//! The user's folders on Android, through the Storage Access Framework.
//!
//! An Android app can't read ChromeOS's folders by path. The user adds a location once (the
//! system folder picker, answered to the Java activity, `android/.../DeckcraftActivity.java`), and
//! from then on the app lists its folders and makes files in it through the activity. DeckCraft
//! itself works on paths, so each document gets a working copy in the app's storage: opening
//! copies the document in; whatever is saved to the copy is copied back to the document. The
//! locations, recent folders and the copy→document map are kept on disk, so a document from
//! Recent still saves back after a restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};
use serde::{Deserialize, Serialize};

/// Folder picker answers waiting for the next frame: (location uri, display name).
static PICKED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();
/// The Java activity. ndk_context holds the Application instead, which has none of its methods.
static ACTIVITY: AtomicUsize = AtomicUsize::new(0);

pub fn set_activity(app: &winit::platform::android::activity::AndroidApp) {
    // Set on every start: android_main can run again with a new activity.
    ACTIVITY.store(app.activity_as_ptr() as usize, Ordering::SeqCst);
}

pub fn set_context(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_deckcraft_DeckcraftActivity_nativePicked(
    mut env: JNIEnv,
    _class: JClass,
    _kind: jint,
    uri: JString,
    name: JString,
) {
    if uri.is_null() {
        return;
    }
    let Ok(uri) = env.get_string(&uri).map(String::from) else { return };
    let name = if name.is_null() { None } else { env.get_string(&name).ok().map(String::from) };
    if let Ok(mut q) = PICKED.lock() {
        q.push((uri, name.unwrap_or_else(|| "Folder".into())));
    }
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// Calls a method of the activity with string arguments; its string answer (empty for none).
/// `None` when the call failed (logged). Every Java reference made is freed before returning.
fn call(method: &str, sig: &str, strings: &[&str]) -> Option<String> {
    let activity = ACTIVITY.load(Ordering::SeqCst);
    if activity == 0 {
        return None;
    }
    let ctx = ndk_context::android_context();
    // SAFETY: android-activity fills ndk_context with the process's JavaVM.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread_permanently().ok()?;
    let r = env.with_local_frame(16, |env| -> jni::errors::Result<Option<String>> {
        // SAFETY: NativeActivity holds this global reference to the activity for the life of the app.
        let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
        let mut objs = Vec::new();
        for s in strings {
            objs.push(JObject::from(env.new_string(s)?));
        }
        let args: Vec<JValue> = objs.iter().map(JValue::Object).collect();
        let v = match env.call_method(&activity, method, sig, &args) {
            Ok(v) => v,
            Err(e) => {
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_describe();
                    let _ = env.exception_clear();
                }
                log::error!("{method}: {e}");
                return Ok(None);
            }
        };
        Ok(Some(match v.l() {
            Ok(obj) if !obj.is_null() => env.get_string(&JString::from(obj)).map(String::from).unwrap_or_default(),
            _ => String::new(),
        }))
    });
    r.ok().flatten()
}

fn sig(args: usize, returns: bool) -> String {
    format!("({}){}", "Ljava/lang/String;".repeat(args), if returns { "Ljava/lang/String;" } else { "V" })
}

/// A call whose answer starts with '+' (the value follows) or '!' (an error follows).
fn ask(method: &str, strings: &[&str]) -> Result<String, String> {
    let a = call(method, &sig(strings.len(), true), strings).ok_or("Android didn't answer")?;
    match a.split_at_checked(1) {
        Some(("+", v)) => Ok(v.to_string()),
        Some(("!", e)) => Err(e.to_string()),
        _ => Err("Android gave no answer".into()),
    }
}

/// `copyIn` / `copyOut`: empty answer = done.
fn copy(method: &str, a: &str, b: &str) -> Result<(), String> {
    match call(method, &sig(2, true), &[a, b]) {
        Some(e) if e.is_empty() => Ok(()),
        Some(e) => Err(e),
        None => Err("Android didn't answer".into()),
    }
}

fn doc_uri(tree: &str, doc: &str) -> Result<String, String> {
    call("docUri", &sig(2, true), &[tree, doc]).filter(|u| !u.is_empty()).ok_or_else(|| "Android didn't answer".to_string())
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    std::fs::metadata(path).ok().and_then(|m| Some((m.modified().ok()?, m.len())))
}

/// Browser ids carry the location and the document: `tree` + unit separator + `doc`.
fn join(tree: &str, doc: &str) -> String {
    format!("{tree}\u{1f}{doc}")
}

fn split(id: &str) -> Result<(&str, &str), String> {
    id.split_once('\u{1f}').ok_or_else(|| "unknown folder".to_string())
}

/// A location, folder or file in the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// For a recent folder: the location it is in.
    pub detail: String,
    pub folder: bool,
}

/// A location the user let the app use (a folder tree), or a folder in one used recently.
#[derive(Clone, Serialize, Deserialize)]
struct Place {
    tree: String,
    /// The folder's document id inside `tree` (for a location, its top folder).
    doc: String,
    name: String,
    #[serde(default)]
    location: String,
}

impl Place {
    fn item(&self) -> Item {
        Item { id: join(&self.tree, &self.doc), name: self.name.clone(), detail: self.location.clone(), folder: true }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Places {
    places: Vec<Place>,
    recent: Vec<Place>,
}

/// A Save As the app is writing: where it goes once the working copy is written.
struct Pending {
    folder: String,
    folder_name: String,
    name: String,
    /// The file of that name to replace.
    existing: Option<String>,
    /// The working copy as last seen: written once it stops changing.
    seen: Option<(SystemTime, u64)>,
}

/// Locations, recent folders, and the working copies with the documents they belong to.
pub struct Files {
    dir: Option<PathBuf>,
    places: Places,
    /// working copy path → document uri (persisted in `docs.json`)
    docs: HashMap<String, String>,
    /// working copy path → what it was when last copied back
    synced: HashMap<String, Option<(SystemTime, u64)>>,
    pending: HashMap<String, Pending>,
    /// Folders handed to the app to write into (exporting pages): local folder → (folder id,
    /// folder name, each file there as last seen, the files already put in the folder).
    out_dirs: HashMap<String, OutDir>,
    last_check: Instant,
}

/// A local folder the app writes files into, each copied to the user's folder once written.
struct OutDir {
    folder: String,
    folder_name: String,
    seen: HashMap<String, Option<(SystemTime, u64)>>,
    done: std::collections::HashSet<String>,
}

impl Files {
    pub fn new(data_dir: Option<PathBuf>) -> Files {
        let dir = data_dir.map(|d| d.join("docs"));
        let read = |name: &str| dir.as_ref().and_then(|d| std::fs::read(d.join(name)).ok());
        let docs: HashMap<String, String> = read("docs.json").and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let places = read("places.json").and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let synced = docs.keys().map(|p| (p.clone(), stamp(Path::new(p)))).collect();
        Files { dir, places, docs, synced, pending: HashMap::new(), out_dirs: HashMap::new(), last_check: Instant::now() }
    }

    fn store(&self) {
        let Some(d) = &self.dir else { return };
        let _ = std::fs::create_dir_all(d);
        if let Ok(b) = serde_json::to_vec_pretty(&self.docs) {
            let _ = std::fs::write(d.join("docs.json"), b);
        }
        if let Ok(b) = serde_json::to_vec_pretty(&self.places) {
            let _ = std::fs::write(d.join("places.json"), b);
        }
    }

    /// A working copy path for `key` (a document uri, or a new save), named `name`.
    fn copy_path(&self, key: &str, name: &str) -> Option<PathBuf> {
        if let Some((p, _)) = self.docs.iter().find(|(_, u)| *u == key) {
            return Some(PathBuf::from(p));
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in key.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        let name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
        let folder = self.dir.as_ref()?.join(format!("{h:016x}"));
        std::fs::create_dir_all(&folder).ok()?;
        Some(folder.join(name))
    }

    /// Puts a folder at the top of the recent folders.
    fn used(&mut self, folder: &str, name: &str) {
        let Ok((tree, doc)) = split(folder) else { return };
        let location = self.places.places.iter().find(|p| p.tree == tree).map(|p| p.name.clone()).unwrap_or_default();
        self.places.recent.retain(|p| !(p.tree == tree && p.doc == doc));
        self.places.recent.insert(0, Place { tree: tree.into(), doc: doc.into(), name: name.into(), location });
        self.places.recent.truncate(8);
    }

    pub fn places(&self) -> Vec<Item> {
        self.places.places.iter().map(Place::item).collect()
    }

    pub fn recent(&self) -> Vec<Item> {
        self.places.recent.iter().map(Place::item).collect()
    }

    /// Asks the user for another location; it appears in `places` once chosen.
    pub fn add_place() {
        call("pickTree", "()V", &[]);
    }

    /// A folder's subfolders and files.
    pub fn list(folder: &str) -> Result<Vec<Item>, String> {
        let (tree, doc) = split(folder)?;
        let list: Vec<(String, String, bool)> = serde_json::from_str(&ask("list", &[tree, doc])?).map_err(|e| e.to_string())?;
        Ok(list.into_iter().map(|(id, name, folder)| Item { id: join(tree, &id), name, detail: String::new(), folder }).collect())
    }

    /// A local working copy of `file` (in `folder`, called `folder_name`) to open; saving it saves
    /// the file.
    pub fn open(&mut self, folder: &str, folder_name: &str, file: &Item) -> Result<String, String> {
        let (tree, doc) = split(&file.id)?;
        let uri = doc_uri(tree, doc)?;
        let path = self.copy_path(&uri, &file.name).ok_or("no room for a working copy")?;
        let p = path.to_string_lossy().into_owned();
        copy("copyIn", &uri, &p)?;
        self.docs.insert(p.clone(), uri);
        self.synced.insert(p.clone(), stamp(&path));
        self.used(folder, folder_name);
        self.store();
        Ok(p)
    }

    /// A local path for the app to write `name` at; once written, it goes to `folder` (replacing
    /// `existing`, the file of that name there).
    pub fn begin_save(&mut self, folder: &str, folder_name: &str, name: &str, existing: Option<&Item>) -> Result<String, String> {
        split(folder)?;
        let key = format!("{folder}\u{1f}{name}\u{1f}{:?}", SystemTime::now());
        let path = self.copy_path(&key, name).ok_or("no room for a working copy")?;
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        self.pending.insert(
            p.clone(),
            Pending {
                folder: folder.into(),
                folder_name: folder_name.into(),
                name: name.into(),
                existing: existing.map(|f| f.id.clone()),
                seen: None,
            },
        );
        Ok(p)
    }

    /// A local folder for the app to write into; every file written there goes to `folder`.
    pub fn begin_folder(&mut self, folder: &str, folder_name: &str) -> Result<String, String> {
        split(folder)?;
        let key = format!("{folder}\u{1f}{:?}", SystemTime::now());
        let path = self.copy_path(&key, "out").ok_or("no room for a working folder")?;
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        let p = path.to_string_lossy().into_owned();
        self.out_dirs.insert(
            p.clone(),
            OutDir { folder: folder.into(), folder_name: folder_name.into(), seen: HashMap::new(), done: Default::default() },
        );
        self.used(folder, folder_name);
        self.store();
        Ok(p)
    }

    /// Copies files that stopped changing in the app's working folders to the user's folders.
    fn drain_out_dirs(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        for (dir, out) in &mut self.out_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
                if out.done.contains(&name) || !path.is_file() {
                    continue;
                }
                let now = stamp(&path);
                if out.seen.get(&name) != Some(&now) {
                    out.seen.insert(name, now);
                    continue;
                }
                out.done.insert(name.clone());
                let result = split(&out.folder).and_then(|(tree, doc)| {
                    let uri = ask("create", &[tree, doc, &name])?;
                    copy("copyOut", &path.to_string_lossy(), &uri)
                });
                if let Err(e) = result {
                    problems.push(format!("{name} couldn't be put in {}: {e}", out.folder_name));
                }
            }
        }
        problems
    }

    /// Reads a file (in `folder`, called `folder_name`) and returns its document uri and bytes.
    pub fn read(&mut self, folder: &str, folder_name: &str, file: &Item) -> Result<(String, Vec<u8>), String> {
        let path = self.open(folder, folder_name, file)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let uri = self.docs.get(&path).cloned().ok_or("the file was lost")?;
        Ok((uri, bytes))
    }

    /// Writes `bytes` as `name` in `folder` (replacing `existing`, the file of that name there);
    /// returns the document's uri.
    pub fn write_new(&mut self, folder: &str, folder_name: &str, name: &str, existing: Option<&Item>, bytes: &[u8]) -> Result<String, String> {
        let (tree, doc) = split(folder)?;
        let uri = match existing {
            Some(f) => {
                let (t, d) = split(&f.id)?;
                doc_uri(t, d)?
            }
            None => ask("create", &[tree, doc, name])?,
        };
        self.write(&uri, name, bytes)?;
        self.used(folder, folder_name);
        self.store();
        Ok(uri)
    }

    /// Writes `bytes` over the document at `uri`.
    pub fn write(&mut self, uri: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.copy_path(uri, name).ok_or("no room for a working copy")?;
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let p = path.to_string_lossy().into_owned();
        copy("copyOut", &p, uri)?;
        self.synced.insert(p.clone(), stamp(&path));
        self.docs.insert(p, uri.to_string());
        Ok(())
    }

    /// Puts a written Save As working copy into its folder.
    fn finish(&mut self, path: &str, p: Pending) -> Result<(), String> {
        let (tree, doc) = split(&p.folder)?;
        let uri = match &p.existing {
            Some(id) => {
                let (t, d) = split(id)?;
                doc_uri(t, d)?
            }
            None => ask("create", &[tree, doc, &p.name])?,
        };
        copy("copyOut", path, &uri)?;
        self.synced.insert(path.to_string(), stamp(Path::new(path)));
        self.docs.insert(path.to_string(), uri);
        self.used(&p.folder, &p.folder_name);
        self.store();
        Ok(())
    }

    /// Each frame: take new locations; every ~0.7 s, put finished Save As copies in their folders
    /// and copy saved working copies back. Returns what went wrong, to show the user.
    pub fn logic(&mut self) -> Vec<String> {
        let picked: Vec<_> = PICKED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        let mut problems = Vec::new();
        for (tree, name) in picked {
            match ask("rootId", &[&tree]) {
                Ok(doc) => {
                    self.places.places.retain(|p| p.tree != tree);
                    self.places.places.push(Place { tree, doc, name, location: String::new() });
                    self.store();
                }
                Err(e) => problems.push(format!("{name}: {e}")),
            }
        }
        if self.last_check.elapsed() < Duration::from_millis(700) {
            return problems;
        }
        self.last_check = Instant::now();
        // A Save As copy is put in its folder once it has stopped changing between two checks.
        let paths: Vec<String> = self.pending.keys().cloned().collect();
        for path in paths {
            let now = stamp(Path::new(&path));
            let Some(p) = self.pending.get_mut(&path) else { continue };
            if now.is_none() || p.seen != now {
                p.seen = now;
                continue;
            }
            if let Some(p) = self.pending.remove(&path) {
                let name = p.name.clone();
                if let Err(e) = self.finish(&path, p) {
                    problems.push(format!("{name} was saved here, but couldn't be put in its folder: {e}"));
                }
            }
        }
        problems.extend(self.drain_out_dirs());
        problems.extend(self.sync_back());
        problems
    }

    /// Copies every working copy saved since the last check back to its document.
    pub fn sync_back(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        for (p, uri) in &self.docs {
            let now = stamp(Path::new(p));
            if now.is_none() || self.synced.get(p).copied().flatten() == now {
                continue;
            }
            self.synced.insert(p.clone(), now);
            if let Err(e) = copy("copyOut", p, uri) {
                problems.push(format!("Saved here, but it couldn't be written back to the original file: {e}"));
            }
        }
        problems
    }
}
