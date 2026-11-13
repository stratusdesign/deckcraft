//! DeckCraft as an Android app (phones, tablets and ChromeOS, which runs Android apps natively).
//!
//! Runs the same [`deckcraft_ui_egui::SlideApp`] as the desktop app through eframe's Android
//! runner (winit NativeActivity, wgpu on Vulkan or GLES). Files go through DeckCraft's own folder
//! browser ([`browser`]) over the folders the user let it use ([`files`]), handed over as bytes the
//! way the web build's host-kept files are: a presentation opened from a folder saves back to it.
//! Build: `.github/workflows/android.yml`.
#![cfg(target_os = "android")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod browser;
mod files;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use browser::{Ask, Done};
use deckcraft_engine::Session;
use deckcraft_ui_egui::{Inbox, InboxEvent, InboxFile, Services, SlideApp};
use winit::platform::android::activity::AndroidApp;

/// The name DeckCraft knows each presentation's file by → that file's document uri.
type Named = Rc<RefCell<HashMap<String, String>>>;

struct App {
    app: SlideApp,
    browser: browser::Browser,
    pending: browser::Pending,
    files: Rc<RefCell<files::Files>>,
    named: Named,
    inbox: Inbox,
}

fn push(inbox: &Inbox, event: InboxEvent) {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(event);
}

impl App {
    fn done(&mut self, done: Done) {
        match done {
            // A presentation opens as the web build's host-kept files do: under its own name, so
            // Save writes it back to where it came from. The app's own request is then over.
            Done::Opened { purpose, id, name, uri, bytes } if purpose == "open" => {
                self.named.borrow_mut().insert(name.clone(), uri);
                push(&self.inbox, InboxEvent::Cancelled { id });
                self.app.open_from_host(Some(&name), &bytes);
            }
            Done::Opened { id, name, bytes, .. } => push(&self.inbox, InboxEvent::File(InboxFile { id, name, bytes })),
            Done::OpenCancelled { id } => push(&self.inbox, InboxEvent::Cancelled { id }),
            Done::Saved { key, name, uri } => {
                if let Some(key) = key {
                    self.named.borrow_mut().insert(key, uri);
                }
                self.app.set_status(format!("Saved {name}"));
            }
            Done::SaveCancelled { name } => self.app.notify(format!("{name} wasn't saved.")),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Documents ChromeOS handed over ("Open with", or DeckCraft as the default app): opened
        // under their own name, so Save writes them back.
        let opened = self.files.borrow_mut().take_opened();
        for o in opened {
            match o.and_then(|(path, uri, name, writable)| std::fs::read(&path).map(|b| (uri, name, writable, b)).map_err(|e| e.to_string())) {
                Ok((uri, name, writable, bytes)) => {
                    // Read only: no file to save back to, so Save asks where to put it.
                    if writable {
                        self.named.borrow_mut().insert(name.clone(), uri);
                    }
                    self.app.open_from_host(Some(&name), &bytes);
                    if !writable {
                        self.app.notify(format!("{name} was opened read only. Save asks where to put your changes."));
                    }
                }
                Err(e) => self.app.notify(e),
            }
        }
        let problems = self.files.borrow_mut().logic();
        if let Some(p) = problems.into_iter().last() {
            self.app.notify(p);
        }
        self.app.logic(ctx);
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        if let Some(done) = self.browser.show(ui.ctx(), &self.pending, &self.files) {
            self.done(done);
        }
    }
    fn on_exit(&mut self) {
        let _ = self.files.borrow_mut().sync_back();
    }
}

/// File services: open through the browser, Save back to the file a presentation came from (or
/// through the browser when it has none), exports through the browser.
fn services(pending: browser::Pending, files: Rc<RefCell<files::Files>>, named: Named, inbox: Inbox, ctx: egui::Context) -> Services {
    let seq = Rc::new(Cell::new(0u64));
    let ask = {
        let ctx = ctx.clone();
        move |a: Ask| {
            seq.set(seq.get() + 1);
            // A new dialog replaces one still open.
            *pending.borrow_mut() = Some((seq.get(), a));
            ctx.request_repaint();
        }
    };
    let ask = Rc::new(ask);
    let open = ask.clone();
    let save = ask.clone();
    Services {
        open_async: Some(Box::new(move |purpose: &str, id: u64| open(Ask::Open { purpose: purpose.to_string(), id }))),
        host_save: Some(Box::new(move |name: &str, bytes: &[u8], save_as: bool| {
            let uri = if save_as { None } else { named.borrow().get(name).cloned() };
            match uri {
                Some(uri) => files.borrow_mut().write(&uri, name, bytes),
                None => {
                    save(Ask::Save { name: name.to_string(), key: Some(name.to_string()), bytes: bytes.to_vec() });
                    Ok(())
                }
            }
        })),
        download: Some(Box::new(move |name: &str, bytes: &[u8]| ask(Ask::Save { name: name.to_string(), key: None, bytes: bytes.to_vec() }))),
        inbox: Some(inbox),
        ..Default::default()
    }
}

#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("deckcraft"));
    files::set_activity(&android_app);
    let data = android_app.internal_data_path();
    let files = Rc::new(RefCell::new(files::Files::new(data)));
    let pending: browser::Pending = Rc::new(RefCell::new(None));
    let named: Named = Rc::default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("DeckCraft"),
        android_app: Some(android_app),
        ..Default::default()
    };
    let result = eframe::run_native(
        "DeckCraft",
        options,
        Box::new(move |cc| {
            files::set_context(&cc.egui_ctx);
            deckcraft_ui_egui::install_ime_pass_hooks(&cc.egui_ctx);
            let inbox = Inbox::default();
            let services = services(pending.clone(), files.clone(), named.clone(), inbox.clone(), cc.egui_ctx.clone());
            let mut app = SlideApp::new(Session::new(), services);
            if let Err(e) = app.start_for_host() {
                log::error!("new presentation: {e}");
            }
            Ok(Box::new(App { app, browser: browser::Browser::default(), pending, files, named, inbox }))
        }),
    );
    if let Err(e) = result {
        log::error!("DeckCraft stopped: {e}");
    }
}
