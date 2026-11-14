//! The Android clipboard, so copy and paste work with other apps. egui-winit has no Android
//! clipboard: it keeps copies in memory, so text never left the app and nothing came in.
//!
//! Copies: each frame's `CopyText` goes to Android as well. Pastes: Ctrl+V makes egui-winit paste
//! its in-memory copy; [`Bridge::input`] swaps in the Android clipboard's text. egui-winit only
//! pastes when its own copy isn't empty, so [`Bridge::output`] keeps a marker there.

use crate::files::call;

/// Kept in egui's in-memory clipboard so Ctrl+V always reaches us. Never handed to Android.
const MARKER: &str = "\u{2063}";

/// The text on the Android clipboard (empty when it holds none); `None` when Android didn't answer.
pub fn get() -> Option<String> {
    call("clipboardText", "()Ljava/lang/String;", &[]).map(|t| t.replace("\r\n", "\n"))
}

fn set(text: &str) {
    let _ = call("setClipboardText", "(Ljava/lang/String;)Ljava/lang/String;", &[text]);
}

#[derive(Default)]
pub struct Bridge {
    marked: bool,
}

impl Bridge {
    /// Before the app sees input: a paste carries the Android clipboard's text.
    pub fn input(&mut self, raw: &mut egui::RawInput) {
        if !raw.events.iter().any(|e| matches!(e, egui::Event::Paste(_))) {
            return;
        }
        let android = get();
        raw.events.retain_mut(|e| {
            let egui::Event::Paste(t) = e else { return true };
            match &android {
                Some(a) if a.is_empty() => false,
                Some(a) => {
                    a.clone_into(t);
                    true
                }
                // Android didn't answer: paste the app's own copy, never the marker.
                None => t != MARKER,
            }
        });
    }

    /// After the frame: hand its copy to Android, and keep the marker in egui's clipboard.
    pub fn output(&mut self, ctx: &egui::Context) {
        let copied = ctx.output(|o| {
            o.commands.iter().rev().find_map(|c| match c {
                egui::OutputCommand::CopyText(t) if t != MARKER => Some(t.clone()),
                _ => None,
            })
        });
        if let Some(t) = &copied {
            set(t);
        }
        if !self.marked || copied.is_some_and(|t| t.is_empty()) {
            ctx.copy_text(MARKER.to_owned());
            self.marked = true;
        }
    }
}
