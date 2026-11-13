//! DeckCraft's file dialogs on Android: one folder browser for Open, Save As, Export and Insert
//! Picture/Audio/Video, drawn over the app. ChromeOS shows its own picker to Android apps only for
//! opening files, so the app browses the user's folders itself and every dialog looks and works the
//! same: locations and recent folders on the left, the folder's contents on the right, and when
//! saving a name box. DeckCraft hands its host files as bytes (as its web build does), so a dialog
//! is an [`Ask`] and ends as a [`Done`] that the app wrapper applies ([`crate::files`] reads and
//! writes the user's folders).

use std::cell::RefCell;
use std::rc::Rc;


use crate::files::{Files, Item};
use crate::page;

/// DeckCraft's colour for the page's left column (GridCraft's is green); see [`page::View::accent`].
const ACCENT: [u32; 3] = [0xC43E1C, 0x4A2A1F, 0xF8E1D9];

const PRESENTATIONS: &[&str] = &["deckcraft", "pptx", "potx", "ppsx", "txt", "md"];
const PICTURES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];
const AUDIO: &[&str] = &["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aif", "aiff"];
const VIDEO: &[&str] = &["mp4", "m4v", "mov", "webm"];

/// A dialog the app asked for.
pub enum Ask {
    /// Open a file for `purpose` (`open`, `picture`, `audio`, `video`); `id` is the app's request.
    Open { purpose: String, id: u64 },
    /// Save `bytes`, suggesting `name`. `key` is the name the app knows the file by, so a later
    /// Save of it goes to the same place.
    Save { name: String, key: Option<String>, bytes: Vec<u8> },
}

/// How a dialog ended.
pub enum Done {
    Opened { purpose: String, id: u64, name: String, uri: String, bytes: Vec<u8> },
    OpenCancelled { id: u64 },
    Saved { key: Option<String>, name: String, uri: String },
    SaveCancelled { name: String },
}

/// The dialog waiting to be shown, numbered so a new one is noticed.
pub type Pending = Rc<RefCell<Option<(u64, Ask)>>>;

/// Where the browser is; it starts afresh for every dialog.
#[derive(Default)]
pub struct Browser {
    /// The dialog being shown (to notice a new one).
    shown: Option<u64>,
    /// The folders from the starting location down to the one shown: (id, name).
    trail: Vec<(String, String)>,
    listing: Option<Result<Vec<Item>, String>>,
    name: String,
    /// The existing file the user has been asked about replacing.
    replace: Option<String>,
    /// Something to tell the user inside the dialog.
    note: Option<String>,
}


/// The file types a dialog lists: `None` lists every file.
fn wanted(ask: &Ask) -> Option<Vec<String>> {
    let exts = match ask {
        Ask::Open { purpose, .. } => match purpose.as_str() {
            "picture" => PICTURES,
            "audio" => AUDIO,
            "video" => VIDEO,
            _ => PRESENTATIONS,
        },
        // Saving lists every file, so a name already taken asks before it is replaced.
        Ask::Save { .. } => return None,
    };
    Some(exts.iter().map(|e| e.to_string()).collect())
}

fn shown(item: &Item, exts: &Option<Vec<String>>) -> bool {
    item.folder
        || exts.as_ref().is_none_or(|exts| {
            std::path::Path::new(&item.name).extension().and_then(std::ffi::OsStr::to_str).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
        })
}

impl Browser {
    fn go(&mut self, trail: Vec<(String, String)>) {
        self.trail = trail;
        self.listing = None;
        self.replace = None;
    }

    /// Draws the dialog the app is waiting on, if any, and answers it.
    pub fn show(&mut self, ctx: &egui::Context, pending: &Pending, files: &Rc<RefCell<Files>>) -> Option<Done> {
        let (seq, saving, exts) = {
            let p = pending.borrow();
            let (seq, ask) = p.as_ref()?;
            (*seq, matches!(ask, Ask::Save { .. }), wanted(ask))
        };
        if self.shown != Some(seq) {
            *self = Browser::default();
            if let Some((_, Ask::Save { name, .. })) = pending.borrow().as_ref() {
                self.name = name.clone();
            }
            self.shown = Some(seq);
        }
        let (places, recent) = {
            let f = files.borrow();
            (f.places(), f.recent())
        };
        if self.trail.is_empty()
            && let Some(start) = recent.first().or(places.first())
        {
            self.go(vec![(start.id.clone(), start.name.clone())]);
        }
        if self.listing.is_none()
            && let Some((id, _)) = self.trail.last()
        {
            self.listing = Some(Files::list(id).map(|mut items| {
                items.retain(|i| shown(i, &exts));
                items.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                items
            }));
        }

        let mut answer: Option<Done> = None;
        let mut cancel = false;
        let mut add_place = false;
        let mut go = None;
        let mut picked = None;
        let mut save_now = false;
        let mut up = None;
        let typed = self.name.clone();
        let view = page::View {
            title: if saving { "Save As" } else { "Open" },
            app: "DeckCraft",
            saving,
            choosing_folder: false,
            places: &places,
            recent: &recent,
            trail: &self.trail,
            listing: self.listing.as_ref(),
            replacing: self.replace.is_some(),
            note: self.note.as_deref(),
            accent: ACCENT,
        };
        let mut name = typed.clone();
        let actions = page::show(ctx, &view, &mut name);
        if name != typed {
            // A different name drops the question about replacing the old one.
            self.replace = None;
        }
        self.name = name;
        for action in actions {
            match action {
                page::Action::Cancel => cancel = true,
                page::Action::AddPlace => add_place = true,
                page::Action::Go(item) => go = Some(item),
                page::Action::Pick(item) => picked = Some(item),
                page::Action::Up(i) => up = Some(i),
                page::Action::Save | page::Action::Replace => save_now = true,
                page::Action::ChooseFolder => {}
            }
        }
        if let Some(i) = up {
            let trail = self.trail[..=i].to_vec();
            self.go(trail);
        }
        if add_place {
            Files::add_place();
        }
        if let Some(place) = go {
            self.go(vec![(place.id, place.name)]);
        }
        if let Some(item) = picked {
            if item.folder {
                let mut trail = self.trail.clone();
                trail.push((item.id, item.name));
                self.go(trail);
            } else if saving {
                self.name = item.name;
                self.replace = None;
            } else {
                let (folder, folder_name) = self.trail.last().cloned().unwrap_or_default();
                match files.borrow_mut().read(&folder, &folder_name, &item) {
                    Ok((uri, bytes)) => {
                        if let Some((_, Ask::Open { purpose, id })) = pending.borrow().as_ref() {
                            answer = Some(Done::Opened { purpose: purpose.clone(), id: *id, name: item.name.clone(), uri, bytes });
                        }
                    }
                    Err(e) => self.note = Some(format!("{}: {e}", item.name)),
                }
            }
        }
        if save_now {
            let name = self.name.trim().to_string();
            let existing = match &self.listing {
                Some(Ok(items)) => items.iter().find(|i| !i.folder && i.name == name).cloned(),
                _ => None,
            };
            if let Some(e) = &existing
                && self.replace.as_deref() != Some(e.id.as_str())
            {
                self.replace = Some(e.id.clone());
            } else if let Some((folder, folder_name)) = self.trail.last().cloned() {
                let written = match pending.borrow().as_ref() {
                    Some((_, Ask::Save { key, bytes, .. })) => {
                        Some(files.borrow_mut().write_new(&folder, &folder_name, &name, existing.as_ref(), bytes).map(|uri| (key.clone(), uri)))
                    }
                    _ => None,
                };
                match written {
                    Some(Ok((key, uri))) => answer = Some(Done::Saved { key, name: name.clone(), uri }),
                    Some(Err(e)) => self.note = Some(format!("{name}: {e}")),
                    None => {}
                }
            }
        }
        if answer.is_none() && cancel {
            answer = pending.borrow().as_ref().map(|(_, ask)| match ask {
                Ask::Open { id, .. } => Done::OpenCancelled { id: *id },
                Ask::Save { name, .. } => Done::SaveCancelled { name: name.clone() },
            });
        }
        if answer.is_some() {
            pending.borrow_mut().take();
            ctx.request_repaint();
        }
        answer
    }
}
