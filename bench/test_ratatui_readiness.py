import unittest
from ratatui_readiness import Invalid, Milestones, RatatuiScreen, browser_frame, human_size, loading_frame

EXPECTED = dict(allocated=2048, apparent=9000, files=4, directories=3, errors=0)

def browser(selected=1, total='2.0 KiB', files=4, mode='READ ONLY'):
    lines=[' '*160 for _ in range(40)]
    values={0:' rdu '+mode,1:' Path: /owned/fixture',2:'┌ Scan complete ┐',3:f'│Here {total}   Files {files}   Dirs 3   Errors 0   Scan 0.12s│',5:'┌ Files · allocated · sort:size ┐',6:'│Name      Size      Relative    Kind',38:f'{selected} / 2 · Filter: ',39:'↑↓ Select  Enter Open  / Filter  a Metric  s/n/c Sort  Space Stage  b Basket  ? Help  q Quit'}
    for row,text in values.items():lines[row]=text.ljust(160)
    return tuple(lines)

def loading(entries,active=1,phase='Scanning'):
    lines=[' '*160 for _ in range(40)]
    for row,text in {0:'rdu  SCANNING  0.1s',1:'Path: /owned/fixture',3:'┌ Live metadata progress ┐',4:'│Phase '+phase,6:f'│Entries observed {entries}',7:f'│Files observed {entries}',8:'│Directories completed 0 / discovered 3',9:f'│Active workers {active}',10:'│Errors 0'}.items():lines[row]=text.ljust(160)
    return tuple(lines)

def paint(lines):
    return b''.join(f'\x1b[{i+1};1H'.encode()+line.encode()for i,line in enumerate(lines))+b'\x1b[39m\x1b[49m\x1b[0m\x1b[?25l'

class FrameTests(unittest.TestCase):
    def test_setup_hide_is_not_a_complete_draw_and_split_tail_is(self):
        screen=RatatuiScreen();screen.feed(b'\x1b[?1049h\x1b[?25l');self.assertEqual(len(screen.frames),0)
        data=paint(browser());screen.feed(data[:-1]);self.assertEqual(len(screen.frames),0);screen.feed(data[-1:]);self.assertEqual(len(screen.frames),1);self.assertEqual(browser_frame(screen.frames[0][1],EXPECTED)['selected'],1)
    def test_marker_without_immediately_preceding_reset_does_not_complete(self):
        screen=RatatuiScreen();screen.feed(b'\x1b[?1049h\x1b[0mnot-a-flushed-draw\x1b[?25l');self.assertEqual(len(screen.frames),0)
    def test_unknown_protocol_and_clipboard_and_changed_sync_contract_rejected(self):
        for payload in [b'\x1b[999z',b'\x1b]52;c;fixture\x07',b'\x1b[?2026h']:
            with self.assertRaises(ValueError):RatatuiScreen().feed(payload)
    def test_incomplete_protocol_at_exit_rejected(self):
        screen=RatatuiScreen();screen.feed(b'\x1b[')
        with self.assertRaises(Invalid):screen.finish()
    def test_rounded_metric_and_exact_counts_are_both_required(self):
        self.assertEqual(human_size(2048),'2.0 KiB')
        for frame in [browser(total='2.1 KiB'),browser(files=5),browser(mode='PARTIAL · READ ONLY')]:
            with self.assertRaises(Invalid):browser_frame(frame,EXPECTED)
    def test_writable_case_requires_real_live_staging_binding(self):
        self.assertIsNotNone(browser_frame(browser(mode='LIVE · STAGING'),EXPECTED,True))
        with self.assertRaises(Invalid):browser_frame(browser(),EXPECTED,True)
    def test_incomplete_navigation_footer_cannot_be_ready(self):
        frame=list(browser());frame[39]=' '*160
        with self.assertRaises(Invalid):browser_frame(frame,EXPECTED)
    def test_first_frame_is_only_eligible_after_fresh_j_k_responses(self):
        state=Milestones(EXPECTED);self.assertEqual(state.observe(2,browser(),.1),b'j')
        with self.assertRaises(Invalid):state.result()
        state.sent(b'j',.11,3);self.assertIsNone(state.observe(3,browser(2),.12))
        self.assertEqual(state.observe(4,browser(2),.13),b'k');state.sent(b'k',.14,4);self.assertEqual(state.observe(5,browser(),.15),b'q')
        result=state.result();self.assertEqual(result['first_usable_browser_seconds'],.1);self.assertTrue(result['navigation_round_trip_validated']);self.assertFalse(result['hidden_exact_bytes_verified_by_screen'])
    def test_positive_progress_is_not_a_fabricated_three_update_claim(self):
        state=Milestones(EXPECTED);state.observe(1,loading(0),.01);state.observe(2,loading(5),.02)
        state.observe(3,browser(),.1);state.sent(b'j',.11,3);state.observe(4,browser(2),.12);state.sent(b'k',.13,4);state.observe(5,browser(),.14)
        self.assertFalse(state.result()['live_progress_three_updates_observable'])
    def test_three_live_updates_and_decreasing_counter_negative(self):
        state=Milestones(EXPECTED)
        for serial,value in enumerate((1,2,3),1):state.observe(serial,loading(value),serial/10)
        self.assertEqual(len(state.active_samples),3)
        with self.assertRaises(Invalid):state.observe(4,loading(2),.4)
    def test_navigation_must_not_change_the_root(self):
        state=Milestones(EXPECTED);state.observe(1,browser(),.1);state.sent(b'j',.11,1);frame=list(browser(2));frame[1]=' Path: /owned/different'.ljust(160)
        with self.assertRaises(Invalid):state.observe(2,frame,.12)


class RestorationTests(unittest.TestCase):
    def test_master_positive_observation_can_stand_when_slave_unavailable(self):
        from ratatui_readiness import restoration
        before={'master':{'available':True,'value':[1,2,3]},'slave':{'available':True,'value':[1,2,3]}}
        after={'master':{'available':True,'value':[1,2,3]},'slave':{'available':False,'errno':25}}
        observed=restoration(before,after)
        self.assertTrue(observed['restored']);self.assertTrue(observed['observations']['master']['equal']);self.assertIsNone(observed['observations']['slave']['equal'])
    def test_no_valid_descriptor_is_unavailable_not_pass_or_proven_mismatch(self):
        from ratatui_readiness import restoration
        before={name:{'available':False,'errno':25}for name in ('master','slave')}
        self.assertIsNone(restoration(before,before)['restored'])
    def test_any_observed_mismatch_invalidates_cleanup(self):
        from ratatui_readiness import restoration
        before={name:{'available':True,'value':[1,2,3]}for name in ('master','slave')}
        after={name:{'available':True,'value':[1,2,4]}for name in ('master','slave')}
        self.assertFalse(restoration(before,after)['restored'])


class BindingNegatives(unittest.TestCase):
    def test_unknown_or_changed_geometry_contract_rejected_before_any_binary_read(self):
        from ratatui_readiness import validate_binding
        from pathlib import Path
        with self.assertRaises(Invalid):validate_binding({},Path('/unused'),{'format':'old-spinner'},Path('/unused'))
    def test_boolean_gate_and_wrong_variant_gate_are_not_evidence(self):
        from ratatui_readiness import validate_external_gates
        binding={'source_commit':'f'*40,'binary_sha256':'a'*64}
        for gates in [{'exact_cli':True,'retained_tree':True},{'exact_cli':{'passed':True,'source_commit':'0'*40,'binary_sha256':'a'*64}}]:
            with self.assertRaises(Invalid):validate_external_gates(gates,binding)
    def test_changed_external_gate_file_rejected(self):
        from ratatui_readiness import validate_external_gates
        import tempfile
        from pathlib import Path
        binding={'source_commit':'f'*40,'binary_sha256':'a'*64}
        with tempfile.TemporaryDirectory() as directory:
            receipt=Path(directory)/'receipt.json';receipt.write_text('{}')
            gate=dict(binding,passed=True,receipt=str(receipt),sha256='0'*64)
            with self.assertRaises(Invalid):validate_external_gates({'exact_cli':gate,'retained_tree':gate},binding)

if __name__=='__main__':unittest.main()
