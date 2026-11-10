use super::{Action, App, UiOptions, render};
use crate::delete::{
    self, DeleteControl, DeleteReport, DeletionSession, PrepareControl, PreparedPlan,
};
use crate::scan::{self, Options, Tree};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::{
        cursor::{Hide, Show},
        event::{self, Event, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

static STOP: AtomicI32 = AtomicI32::new(0);
extern "C" fn stop_signal(signal: libc::c_int) {
    STOP.store(signal, Ordering::Relaxed);
}

struct Session {
    raw: bool,
    screen: bool,
    signals: Vec<(i32, libc::sigaction)>,
}
impl Session {
    fn enter() -> io::Result<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "interactive mode requires a terminal on stdin and stdout",
            ));
        }
        let mut guard = Self {
            raw: false,
            screen: false,
            signals: Vec::new(),
        };
        STOP.store(0, Ordering::Relaxed);
        for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = stop_signal as *const () as libc::sighandler_t;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            let mut old = std::mem::MaybeUninit::uninit();
            if unsafe { libc::sigaction(signal, &action, old.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            guard.signals.push((signal, unsafe { old.assume_init() }));
        }
        enable_raw_mode()?;
        guard.raw = true;
        // Mark before the fallible write, so a partially emitted setup is undone.
        guard.screen = true;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        static PANIC: std::sync::Once = std::sync::Once::new();
        let owner = std::thread::current().id();
        PANIC.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if std::thread::current().id() == owner {
                    let _ = disable_raw_mode();
                    let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
                }
                previous(info);
            }));
        });
        Ok(guard)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if self.raw {
            let _ = disable_raw_mode();
        }
        if self.screen {
            let _ = execute!(
                io::stdout(),
                ratatui::crossterm::style::ResetColor,
                Show,
                LeaveAlternateScreen
            );
        }
        for (signal, old) in self.signals.iter().rev() {
            unsafe {
                libc::sigaction(*signal, old, std::ptr::null_mut());
            }
        }
    }
}

enum Message {
    Input(io::Result<Event>),
    Finished(io::Result<(Tree, Option<DeletionSession>)>),
    Prepared {
        generation: u64,
        result: io::Result<PreparedPlan>,
    },
    Deleted(DeleteReport),
}
struct Input {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}
impl Input {
    fn start(sender: mpsc::SyncSender<Message>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let handle = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                match event::poll(Duration::from_millis(40)) {
                    Ok(true) => {
                        if matches!(
                            sender.try_send(Message::Input(event::read())),
                            Err(mpsc::TrySendError::Disconnected(_))
                        ) {
                            break;
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        let _ = sender.try_send(Message::Input(Err(e)));
                        break;
                    }
                }
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
fn options(mut ui: UiOptions) -> UiOptions {
    ui.no_color |= std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    ui
}
fn validate(tree: &Tree) -> io::Result<()> {
    if tree.dirs.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "scan contains no root directory",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn run_snapshot(tree: Tree, ui: UiOptions) -> io::Result<u8> {
    run_borrowed(&tree, ui)?;
    Ok(if tree.errors > 0 || !tree.complete {
        2
    } else {
        0
    })
}
pub(super) fn run_borrowed(tree: &Tree, ui: UiOptions) -> io::Result<()> {
    validate(tree)?;
    let _session = Session::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (tx, rx) = mpsc::sync_channel(256);
    let _input = Input::start(tx);
    let mut app = App::new(options(ui));
    app.imported = tree.origin == scan::TreeOrigin::Imported;
    app.partial = !tree.complete || tree.errors > 0;
    app.rebuild(tree);
    loop {
        terminal.draw(|f| render::browser(f, tree, &mut app))?;
        if STOP.load(Ordering::Relaxed) != 0 {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Message::Input(Ok(Event::Key(key)))) if key.kind != KeyEventKind::Release => {
                match app.key(tree, key, terminal.size()?.height) {
                    Action::Quit => break,
                    Action::Refresh => {
                        app.message = "Offline snapshot: start a live scan to refresh".into()
                    }
                    Action::Stage | Action::Unstage | Action::Review | Action::StopPreparation => {
                        app.message = "READ ONLY: staging disabled".into()
                    }
                    Action::None => {}
                }
            }
            Ok(Message::Input(Ok(Event::Resize(..)))) => terminal.clear()?,
            Ok(Message::Input(Err(e))) => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn run_path(path: PathBuf, mut options: Options, ui: UiOptions) -> io::Result<u8> {
    let _session = Session::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (tx, rx) = mpsc::sync_channel(256);
    let _input = Input::start(tx.clone());
    let mut app = App::new(self::options(ui));
    let mut tree: Option<Tree> = None;
    let mut progress = Arc::new(scan::ScanProgress::new());
    options.progress = Some(progress.clone());
    let mut worker = Some(start_scan(
        &tx,
        path.clone(),
        options.clone(),
        app.options.read_only,
    ));
    let mut preparation: Option<Preparation> = None;
    let mut execution: Option<std::thread::JoinHandle<()>> = None;
    let mut generation = 0u64;
    let mut rates = Vec::new();
    let mut sample = (Instant::now(), 0u64);
    let result = (|| {
        loop {
            if app.deleting.is_some() {
                terminal.draw(|f| render::deleting(f, &app))?;
            } else if app.report.is_some() {
                terminal.draw(|f| render::report(f, &app))?;
            } else if let Some(mut reader) = app.manifest.take() {
                let count = app
                    .prepared
                    .as_ref()
                    .map_or(0, |p| p.summary().object_count);
                terminal
                    .draw(|f| render::manifest(f, &app, &mut reader, app.manifest_index, count))?;
                app.manifest = Some(reader);
            } else if let Some(mut review) = app.review.take() {
                terminal.draw(|f| render::review(f, &app, &mut review))?;
                app.review = Some(review);
            } else if let Some(tree) = &tree {
                terminal.draw(|f| render::browser(f, tree, &mut app))?;
            } else {
                let snapshot = progress.snapshot();
                if sample.0.elapsed() >= Duration::from_millis(100) {
                    let rate = ((snapshot.entries_seen.saturating_sub(sample.1)) as f64
                        / sample.0.elapsed().as_secs_f64()) as u64;
                    rates.push(rate);
                    if rates.len() > 96 {
                        rates.remove(0);
                    }
                    sample = (Instant::now(), snapshot.entries_seen);
                }
                let counters = format!(
                    "Phase {:?}{}\n\nEntries observed {}\nFiles observed {}\nDirectories completed {} / discovered {}\nActive workers {}\nErrors {}",
                    snapshot.phase,
                    if progress.is_cancelled() {
                        " | stopping; keeping partial results"
                    } else {
                        ""
                    },
                    snapshot.entries_seen,
                    snapshot.files_seen,
                    snapshot.directories_completed,
                    snapshot.directories_discovered,
                    snapshot.active_workers,
                    snapshot.errors
                );
                terminal.draw(|f| {
                    render::loading(
                        f,
                        &app,
                        &path,
                        snapshot.elapsed.as_secs_f64(),
                        &counters,
                        &rates,
                    )
                })?;
            }
            if STOP.load(Ordering::Relaxed) != 0 {
                if let Some(control) = &app.deleting {
                    control.cancel();
                } else {
                    break;
                }
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Message::Finished(result)) => {
                    if let Some(worker) = worker.take() {
                        let _ = worker.join();
                    }
                    let (scanned, session) = result?;
                    validate(&scanned)?;
                    app.partial = !scanned.complete || scanned.errors > 0;
                    app.rebuild(&scanned);
                    app.session = session.map(Arc::new);
                    tree = Some(scanned);
                }
                Ok(Message::Prepared {
                    generation: prepared_generation,
                    result,
                }) => {
                    if prepared_generation == generation {
                        if let Some((_, handle)) = preparation.take() {
                            let _ = handle.join();
                        }
                        app.preparing = false;
                        match result {
                            Ok(plan) => {
                                app.message = format!(
                                    "{} targets freshly prepared | b Basket | D Final review",
                                    plan.summary().target_count
                                );
                                app.prepared = Some(plan);
                                app.open_review();
                            }
                            Err(error) => {
                                app.prepared = None;
                                app.prepare_failed = true;
                                app.message = format!(
                                    "Preparation refused: {}",
                                    super::safe_text(std::ffi::OsStr::new(&error.to_string()))
                                );
                            }
                        }
                    }
                }
                Ok(Message::Deleted(report)) => {
                    if let Some(handle) = execution.take() {
                        let _ = handle.join();
                    }
                    app.deleting = None;
                    app.session = None;
                    app.prepared = None;
                    app.clear_staged();
                    app.partial = true;
                    app.report = Some(report);
                    app.report_scroll = 0;
                }
                Ok(Message::Input(Ok(Event::Key(key)))) if key.kind != KeyEventKind::Release => {
                    if let Some(control) = &app.deleting {
                        if matches!(
                            key.code,
                            event::KeyCode::Char('x' | 'q') | event::KeyCode::Esc
                        ) || key.modifiers.contains(event::KeyModifiers::CONTROL)
                        {
                            control.cancel();
                        }
                        continue;
                    }
                    if app.manifest.is_some() {
                        manifest_key(&mut app, key.code);
                        continue;
                    }
                    if app.review.is_some() {
                        let size = terminal.size()?;
                        app.review.as_mut().unwrap().layout(size.width, size.height);
                        if key.code == event::KeyCode::Char('m') {
                            open_manifest(&mut app, 0);
                            continue;
                        }
                        let decision = if key.modifiers.contains(event::KeyModifiers::CONTROL)
                            && key.code == event::KeyCode::Char('c')
                        {
                            super::review::ReviewAction::Cancel
                        } else {
                            app.review.as_mut().unwrap().key(key.code)
                        };
                        match decision {
                            super::review::ReviewAction::Cancel => {
                                app.review = None;
                                app.prepared = None;
                                app.message =
                                    "Final review cancelled; prepared plan discarded".into();
                            }
                            super::review::ReviewAction::Confirm => {
                                let phrase = app.review.take().unwrap().typed;
                                if let Some(plan) = app.prepared.take() {
                                    match plan.confirm(&phrase) {
                                        Ok(confirmed) => {
                                            let control = Arc::new(DeleteControl::default());
                                            app.deleting = Some(control.clone());
                                            app.session = None;
                                            let sender = tx.clone();
                                            execution = Some(std::thread::spawn(move || {
                                                let report=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||delete::execute(confirmed,&control))).unwrap_or_else(|_|DeleteReport{error:Some("Execution panicked: outcome is uncertain. Preserve recovery storage and reconcile its journal; do not retry automatically.".into()),..Default::default()});
                                                let _ = sender.send(Message::Deleted(report));
                                            }));
                                        }
                                        Err(error) => {
                                            app.message = format!("Confirmation refused: {error}")
                                        }
                                    }
                                }
                            }
                            super::review::ReviewAction::None => {}
                        }
                        continue;
                    }
                    let mut refresh = false;
                    if app.report.is_some() {
                        match key.code {
                            event::KeyCode::Char('q') | event::KeyCode::Esc => break,
                            event::KeyCode::Char('r') => refresh = true,
                            event::KeyCode::Down => {
                                app.report_scroll = app.report_scroll.saturating_add(1)
                            }
                            event::KeyCode::Up => {
                                app.report_scroll = app.report_scroll.saturating_sub(1)
                            }
                            event::KeyCode::PageDown => {
                                app.report_scroll = app.report_scroll.saturating_add(
                                    terminal.size()?.height.saturating_sub(8) as usize,
                                )
                            }
                            event::KeyCode::PageUp => {
                                app.report_scroll = app.report_scroll.saturating_sub(
                                    terminal.size()?.height.saturating_sub(8) as usize,
                                )
                            }
                            _ => {}
                        }
                    } else if let Some(current) = &tree {
                        match app.key(current, key, terminal.size()?.height) {
                            Action::Quit => break,
                            Action::Stage | Action::Unstage => {
                                if app.preparing {
                                    generation = generation.wrapping_add(1);
                                    cancel_preparation(&mut preparation);
                                    app.preparing = false;
                                }
                                app.change_stage(current, app.drawer == super::Drawer::Basket);
                            }
                            Action::StopPreparation => {
                                generation = generation.wrapping_add(1);
                                cancel_preparation(&mut preparation);
                                app.preparing = false;
                                app.message =
                                    "Preparation cancelled; staged basket preserved".into();
                            }
                            Action::Review => {
                                if let Some(targets) = app.start_prepare() {
                                    generation = generation.wrapping_add(1);
                                    let session = app.session.as_ref().unwrap().clone();
                                    let sender = tx.clone();
                                    cancel_preparation(&mut preparation);
                                    let control = Arc::new(PrepareControl::default());
                                    let worker_control = control.clone();
                                    let handle = std::thread::spawn(move || {
                                        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||session.prepare_with_control(&targets,&worker_control))).unwrap_or_else(|_|Err(io::Error::other("preparation worker panicked; no confirmation was issued")));
                                        send_prepared(
                                            &sender,
                                            Message::Prepared { generation, result },
                                            &worker_control,
                                        );
                                    });
                                    preparation = Some((control, handle));
                                }
                            }
                            Action::Refresh => refresh = true,
                            Action::None => {}
                        }
                    } else {
                        match key.code {
                            event::KeyCode::Char('x') => progress.cancel(),
                            event::KeyCode::Char('q') | event::KeyCode::Esc => break,
                            event::KeyCode::Char('c' | 'd')
                                if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                            {
                                break;
                            }
                            _ => {}
                        }
                    }
                    if refresh {
                        generation = generation.wrapping_add(1);
                        cancel_preparation(&mut preparation);
                        tree = None;
                        app = App::new(app.options.clone());
                        progress = Arc::new(scan::ScanProgress::new());
                        options.progress = Some(progress.clone());
                        worker = Some(start_scan(
                            &tx,
                            path.clone(),
                            options.clone(),
                            app.options.read_only,
                        ));
                        rates.clear();
                        sample = (Instant::now(), 0);
                    }
                }
                Ok(Message::Input(Ok(Event::Resize(..)))) => terminal.clear()?,
                Ok(Message::Input(Err(e))) => return Err(e),
                _ => {}
            }
        }
        Ok(
            if app
                .report
                .as_ref()
                .is_some_and(|report| !super::report_is_complete(report))
            {
                2
            } else {
                tree.as_ref()
                    .map_or(0, |t| if t.errors > 0 || !t.complete { 2 } else { 0 })
            },
        )
    })();
    progress.cancel();
    if let Some(control) = &app.deleting {
        control.cancel();
    }
    // A bounded input queue cannot trap a finishing worker when the renderer exits.
    drop(rx);
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    cancel_preparation(&mut preparation);
    if let Some(handle) = execution {
        let _ = handle.join();
    }
    result
}
fn start_scan(
    tx: &mpsc::SyncSender<Message>,
    path: PathBuf,
    options: Options,
    read_only: bool,
) -> std::thread::JoinHandle<()> {
    let sender = tx.clone();
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            DeletionSession::scan(&path, &options, read_only)
        }))
        .unwrap_or_else(|_| Err(io::Error::other("scan worker panicked")));
        let _ = sender.send(Message::Finished(result));
    })
}
fn open_manifest(app: &mut App, index: usize) {
    if let Some(plan) = &app.prepared
        && let Some(path) = plan.manifest_path(index)
    {
        app.manifest_index = index;
        app.manifest = Some(super::review::Review::new(
            super::safe_text(path.as_os_str()),
            String::new(),
        ));
    }
}
fn manifest_key(app: &mut App, key: event::KeyCode) {
    match key {
        event::KeyCode::Esc | event::KeyCode::Char('m') => app.manifest = None,
        event::KeyCode::Char('n') => {
            let count = app
                .prepared
                .as_ref()
                .map_or(0, |p| p.summary().object_count);
            open_manifest(app, (app.manifest_index + 1).min(count.saturating_sub(1)));
        }
        event::KeyCode::Char('p') => open_manifest(app, app.manifest_index.saturating_sub(1)),
        _ => {
            if let Some(reader) = &mut app.manifest {
                let _ = reader.key(key);
            }
        }
    }
}

type Preparation = (Arc<PrepareControl>, std::thread::JoinHandle<()>);
fn cancel_preparation(preparation: &mut Option<Preparation>) {
    if let Some((control, handle)) = preparation.take() {
        control.cancel();
        let _ = handle.join();
    }
}
fn send_prepared(
    sender: &mpsc::SyncSender<Message>,
    mut message: Message,
    control: &PrepareControl,
) {
    loop {
        if control.is_cancelled() {
            return;
        }
        match sender.try_send(message) {
            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => return,
            Err(mpsc::TrySendError::Full(returned)) => {
                message = returned;
                std::thread::park_timeout(Duration::from_millis(2));
            }
        }
    }
}
