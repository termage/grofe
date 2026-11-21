use super::*;

impl PaneHandle {
    /// tmux's screen-state format variables (`cursor_x`, `wrap_flag`,
    /// `mouse_sgr_flag`, ...), read from the emulator; `None` for a name
    /// that is not one of them.
    pub fn term_var(&self, name: &str) -> Option<String> {
        let emu = lock(&self.emu);
        let mode = *emu.term.mode();
        let flag = |m: TermMode| Some(u8::from(mode.contains(m)).to_string());
        match name {
            // tmux's cursor sits one past the last column after a write
            // there, where alacritty holds it on the column and a wrap pending.
            "cursor_x" => {
                let cursor = &emu.term.grid().cursor;
                Some((cursor.point.column.0 + usize::from(cursor.input_needs_wrap)).to_string())
            }
            "cursor_y" => Some(emu.term.grid().cursor.point.line.0.max(0).to_string()),
            // The cell under the cursor; empty on the spacer of a wide character.
            "cursor_character" => {
                let point = emu.term.grid().cursor.point;
                if emu.term.grid().cursor.input_needs_wrap {
                    return Some(" ".to_string());
                }
                let cell = &emu.term.grid()[point];
                if cell.flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                    return Some(String::new());
                }
                let mut out = cell.c.to_string();
                out.extend(cell.zerowidth().into_iter().flatten());
                Some(out)
            }
            "cursor_flag" => flag(TermMode::SHOW_CURSOR),
            "insert_flag" => flag(TermMode::INSERT),
            "keypad_cursor_flag" => flag(TermMode::APP_CURSOR),
            "keypad_flag" => flag(TermMode::APP_KEYPAD),
            "wrap_flag" => flag(TermMode::LINE_WRAP),
            "origin_flag" => flag(TermMode::ORIGIN),
            // A set of 1000, 1002 or 1003 replaces the others and a reset
            // of any clears them all ([`written`]), as in tmux, so at most
            // one is on.
            "mouse_standard_flag" | "mouse_button_flag" | "mouse_all_flag" => {
                let want = match name {
                    "mouse_standard_flag" => MouseMode::Click,
                    "mouse_button_flag" => MouseMode::Drag,
                    _ => MouseMode::Motion,
                };
                Some(u8::from(mouse_mode(mode) == want).to_string())
            }
            // DECSCUSR as tmux reports it. `default` is the style nothing
            // has set; alacritty cannot tell it from a steady block.
            // ponytail: an explicit steady block (DECSCUSR 2) reads
            // `default`; `cursor_colour` (OSC 12) is not tracked, `none`.
            "cursor_shape" => {
                let style = emu.term.cursor_style();
                Some(
                    match style.shape {
                        VteCursorShape::Underline => "underline",
                        VteCursorShape::Beam => "bar",
                        _ if style.blinking => "block",
                        _ => "default",
                    }
                    .to_string(),
                )
            }
            "cursor_blinking" => Some(u8::from(emu.term.cursor_style().blinking).to_string()),
            "cursor_colour" => Some("none".to_string()),
            "cursor_very_visible" => Some("0".to_string()),
            // The cursor the main screen saved on entering the alternate
            // one; tmux's unset value (-1 as unsigned) outside it.
            // ponytail: alacritty keeps the saved cursor private, so inside
            // the alternate screen there is no answer.
            "alternate_saved_x" | "alternate_saved_y" if !mode.contains(TermMode::ALT_SCREEN) => {
                Some(u32::MAX.to_string())
            }
            "mouse_utf8_flag" => flag(TermMode::UTF8_MOUSE),
            "mouse_sgr_flag" => flag(TermMode::SGR_MOUSE),
            _ => None,
        }
    }
}
