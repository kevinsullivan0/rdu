"""Bounded text-only VT model for owned xterm-256color benchmark PTYs.

Unsupported screen-changing sequences raise rather than certify an uncertain
screen. Completed synchronized-output draws are transient, in-memory snapshots.
"""
import codecs
from collections import deque
import unicodedata


class VTScreen:
    def __init__(self, rows=40, columns=160):
        self.rows, self.columns = rows, columns
        self.grid = [[" "] * columns for _ in range(rows)]
        self.x = self.y = 0
        self.saved = (0, 0)
        self.top, self.bottom = 0, rows - 1
        self.state, self.sequence = "ground", ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("strict")
        self.frames = deque()
        self.in_sync = False
        self.draw_count = 0
        self.alt_screen = False
        self.saw_alt_enter = self.saw_alt_leave = False
        self.primary = None
        self.autowrap, self.wrap, self.insert_mode = True, False, False
        self.origin = False
        self.cursor_visible = True
        self.charsets, self.active_charset = ["B", "B"], 0
        self.charset_slot = 0
        self.tabs = set(range(8, columns, 8))
        self.last_char = " "
        self.bytes_received = 0
        self.characters_consumed = 0

    def lines(self):
        return ["".join(row) for row in self.grid]

    def feed(self, data):
        self.bytes_received += len(data)
        for char in self.decoder.decode(data):
            self.characters_consumed += 1
            if self.state == "ground":
                if char == "\x1b": self.state = "escape"
                elif char == "\r": self.x = 0; self.wrap = False
                elif char in "\n\v\f": self._index(); self.wrap = False
                elif char == "\b": self.x = max(0, self.x - 1); self.wrap = False
                elif char == "\t": self.x = min((tab for tab in self.tabs if tab > self.x), default=self.columns - 1); self.wrap = False
                elif char == "\x0e": self.active_charset = 1
                elif char == "\x0f": self.active_charset = 0
                elif char in "\x00\x07": pass
                elif ord(char) < 32 or 0x7f <= ord(char) < 0xa0:
                    raise ValueError("unsupported VT control code")
                else: self._put(char)
            elif self.state == "escape":
                if char == "[": self.state, self.sequence = "csi", ""
                elif char == "]": self.state, self.sequence = "osc", ""
                elif char in "P_^": raise ValueError("unsupported VT control string")
                elif char in "()*+": self.charset_slot = 0 if char in "(*" else 1; self.state = "charset"
                elif char == "7": self.saved = (self.x, self.y); self.state = "ground"
                elif char == "8": self.x, self.y = self.saved; self.wrap = False; self.state = "ground"
                elif char == "D": self._index(); self.state = "ground"
                elif char == "E": self.x = 0; self._index(); self.state = "ground"
                elif char == "M": self._reverse_index(); self.state = "ground"
                elif char == "H": self.tabs.add(self.x); self.state = "ground"
                elif char in "=>": self.state = "ground"
                else: raise ValueError("unsupported VT escape final " + hex(ord(char)))
            elif self.state == "charset":
                if char not in "B0A": raise ValueError("unsupported VT character set")
                self.charsets[self.charset_slot] = char
                self.state = "ground"
            elif self.state == "csi":
                if "@" <= char <= "~":
                    self._csi(self.sequence, char)
                    self.state, self.sequence = "ground", ""
                else:
                    self.sequence += char
                    if len(self.sequence) > 128: raise ValueError("VT CSI exceeded bound")
            elif self.state == "osc":
                if char == "\x07": self._osc_done()
                elif char == "\x1b": self.state = "osc-escape"
                else:
                    self.sequence += char
                    if len(self.sequence) > 8192: raise ValueError("VT OSC exceeded bound")
            elif self.state == "osc-escape":
                if char != "\\": raise ValueError("invalid VT string terminator")
                self._osc_done()

    def _osc_done(self):
        # These operations do not alter the text cells. They are never executed
        # on the user's terminal; the owned PTY's bytes end at this model.
        if self.sequence.split(";", 1)[0] not in {"0", "1", "2", "4", "8", "9", "10", "11", "12", "52", "104", "110", "111", "112"}:
            raise ValueError("unsupported VT OSC operation")
        self.state, self.sequence = "ground", ""

    def _index(self):
        if self.y == self.bottom:
            self.grid[self.top:self.bottom + 1] = self.grid[self.top + 1:self.bottom + 1] + [[" "] * self.columns]
        else: self.y = min(self.rows - 1, self.y + 1)

    def _reverse_index(self):
        if self.y == self.top:
            self.grid[self.top:self.bottom + 1] = [[" "] * self.columns] + self.grid[self.top:self.bottom]
        else: self.y = max(0, self.y - 1)

    def _put(self, char):
        if self.charsets[self.active_charset] == "0":
            char = {"j":"┘", "k":"┐", "l":"┌", "m":"└", "n":"┼", "q":"─", "t":"├", "u":"┤", "v":"┴", "w":"┬", "x":"│"}.get(char, char)
        if unicodedata.combining(char) or unicodedata.category(char) in {"Mn", "Me"}:
            column = self.x if self.wrap else max(0, self.x - 1)
            self.grid[self.y][column] += char
            return
        width = 2 if unicodedata.east_asian_width(char) in {"W", "F"} else 1
        if self.wrap and self.autowrap:
            self.x = 0; self._index()
        self.wrap = False
        if self.x + width > self.columns:
            if not self.autowrap: return
            self.x = 0; self._index()
        if self.insert_mode:
            row = self.grid[self.y]
            row[self.x:] = ([" "] * width + row[self.x:])[:self.columns - self.x]
        self.grid[self.y][self.x] = char
        if width == 2: self.grid[self.y][self.x + 1] = ""
        self.last_char = char
        self.x += width
        if self.x >= self.columns:
            self.x = self.columns - 1
            self.wrap = True

    def _csi(self, raw, final):
        if final == "m": return  # SGR and xterm keyboard mode change no cells.
        if final == "u" and raw in {">1", "<"}: return  # keyboard protocol stack
        if final == "q" and raw.endswith(" "): return  # cursor shape
        if final == "t" and (raw.startswith(("22;", "23;")) or raw == ">2"): return  # window title stack
        private = ""
        if raw.startswith(("?", ">", "<", "=")):
            private, raw = raw[0], raw[1:]
        if any(char not in "0123456789;" for char in raw):
            raise ValueError("unsupported VT CSI parameters")
        values = [int(value) if value else 0 for value in raw.split(";")]
        n = values[0] or 1
        self.wrap = False
        if final in "hl":
            for mode in values: self._mode(private, mode, final == "h")
            return
        if private:
            if final in "cn" and private in {"?", ">"}: return  # queries do not change cells
            raise ValueError("unsupported private VT CSI")
        if final in "Hf":
            self.y = min(self.bottom if self.origin else self.rows - 1, max(self.top if self.origin else 0, (self.top if self.origin else 0) + (values[0] or 1) - 1))
            self.x = min(self.columns - 1, max(0, ((values[1] if len(values) > 1 else 1) or 1) - 1))
        elif final == "A": self.y = max(self.top if self.origin else 0, self.y - n)
        elif final in "Be": self.y = min(self.bottom if self.origin else self.rows - 1, self.y + n)
        elif final in "Ca": self.x = min(self.columns - 1, self.x + n)
        elif final == "D": self.x = max(0, self.x - n)
        elif final == "E": self.x = 0; self.y = min(self.rows - 1, self.y + n)
        elif final == "F": self.x = 0; self.y = max(0, self.y - n)
        elif final in "G`": self.x = min(self.columns - 1, n - 1)
        elif final == "d": self.y = min(self.rows - 1, n - 1)
        elif final == "J":
            mode = values[0]
            if mode in {2, 3}: self.grid = [[" "] * self.columns for _ in range(self.rows)]
            elif mode == 0:
                self.grid[self.y][self.x:] = [" "] * (self.columns - self.x)
                for row in range(self.y + 1, self.rows): self.grid[row] = [" "] * self.columns
            elif mode == 1:
                for row in range(self.y): self.grid[row] = [" "] * self.columns
                self.grid[self.y][:self.x + 1] = [" "] * (self.x + 1)
            else: raise ValueError("unsupported VT erase display mode")
        elif final == "K":
            start, end = {0:(self.x,self.columns),1:(0,self.x+1),2:(0,self.columns)}.get(values[0],(-1,-1))
            if start < 0: raise ValueError("unsupported VT erase line mode")
            self.grid[self.y][start:end] = [" "] * (end - start)
        elif final in "@PX":
            row = self.grid[self.y]
            n = min(n, self.columns - self.x)
            if final == "@": row[self.x:] = ([" "] * n + row[self.x:])[:self.columns-self.x]
            elif final == "P": row[self.x:] = row[self.x+n:] + [" "] * n
            else: row[self.x:self.x+n] = [" "] * n
        elif final in "LM":
            if self.top <= self.y <= self.bottom:
                n = min(n,self.bottom-self.y+1)
                blanks = [[" "]*self.columns for _ in range(n)]
                if final == "L": self.grid[self.y:self.bottom+1] = blanks + self.grid[self.y:self.bottom+1-n]
                else: self.grid[self.y:self.bottom+1] = self.grid[self.y+n:self.bottom+1] + blanks
        elif final in "ST":
            n = min(n,self.bottom-self.top+1)
            blanks = [[" "]*self.columns for _ in range(n)]
            if final == "S": self.grid[self.top:self.bottom+1] = self.grid[self.top+n:self.bottom+1] + blanks
            else: self.grid[self.top:self.bottom+1] = blanks + self.grid[self.top:self.bottom+1-n]
        elif final == "r":
            top = (values[0] or 1) - 1
            bottom = (values[1] if len(values)>1 and values[1] else self.rows) - 1
            if not 0 <= top < bottom < self.rows: raise ValueError("invalid VT scroll margins")
            self.top, self.bottom = top,bottom
            self.x, self.y = 0,(top if self.origin else 0)
        elif final == "s": self.saved = (self.x,self.y)
        elif final == "u": self.x,self.y = self.saved
        elif final == "b":
            for _ in range(min(n,self.rows*self.columns)): self._put(self.last_char)
        elif final == "g":
            if values[0] == 0: self.tabs.discard(self.x)
            elif values[0] == 3: self.tabs.clear()
            else: raise ValueError("unsupported VT tab clear mode")
        elif final in "cn": pass  # device attributes/status queries do not alter text
        else: raise ValueError("unsupported VT CSI final " + hex(ord(final)))

    def _mode(self, private, mode, enabled):
        if not private and mode == 4: self.insert_mode = enabled
        elif private == "?" and mode == 7: self.autowrap = enabled
        elif private == "?" and mode == 6: self.origin = enabled; self.x,self.y = 0,(self.top if enabled else 0)
        elif private == "?" and mode == 25: self.cursor_visible = enabled
        elif private == "?" and mode in {47,1047,1049}:
            if enabled and not self.alt_screen:
                self.primary = (self.grid,self.x,self.y)
                self.grid = [[" "]*self.columns for _ in range(self.rows)]
                self.x=self.y=0
                self.alt_screen = self.saw_alt_enter = True
            elif not enabled and self.alt_screen:
                self.grid,self.x,self.y = self.primary
                self.alt_screen = False
                self.saw_alt_leave = True
        elif private == "?" and mode == 1048:
            if enabled: self.saved = (self.x,self.y)
            else: self.x,self.y = self.saved
        elif private == "?" and mode == 2026:
            if enabled: self.in_sync = True
            elif self.in_sync:
                self.in_sync = False
                self.draw_count += 1
                self.frames.append((self.draw_count, tuple(self.lines())))
                if len(self.frames) > 128: raise ValueError("unconsumed VT draws exceeded bound")
        elif private == "?" and mode in {1,12,1000,1002,1003,1004,1006,2004,9001}: pass
        else: raise ValueError(f"unsupported VT terminal mode {private}{mode}")
