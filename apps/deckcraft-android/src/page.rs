//! The Open / Save As page, drawn like GridCraft's File › Open and File › Save As pages on Android
//! (gridcraft `crates/ui-egui/src/backstage/browser.rs`), so every Craft app on Android picks and
//! saves files the same way: a full-window page with a coloured column on the left (Back), a large
//! title, then the locations and recent folders beside the folder's contents, and when saving a
//! file name box with Save above them. The same file is in each app's Android crate; keep them
//! identical. Only drawing is here: what a click does is the caller's ([`Action`]).

use egui::epaint::{PathShape, PathStroke};
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::files::Item;

/// Width of the left column (GridCraft's `NAV_W`).
const NAV_W: f32 = 196.0;
/// Width of the content column (GridCraft's `CONTENT_W`).
const CONTENT_W: f32 = 760.0;
/// Width of the locations column (GridCraft's `SIDE_W`).
const SIDE_W: f32 = 230.0;

/// What the user did on the page this frame.
pub enum Action {
    Cancel,
    AddPlace,
    /// Show a location or recent folder.
    Go(Item),
    /// A folder or file in the listing was clicked.
    Pick(Item),
    /// Go back up to this folder of the trail.
    Up(usize),
    Save,
    Replace,
    ChooseFolder,
}

/// What the page shows.
pub struct View<'a> {
    pub title: &'a str,
    /// The app's name, for "a folder WordCraft may use".
    pub app: &'a str,
    pub saving: bool,
    pub choosing_folder: bool,
    pub places: &'a [Item],
    pub recent: &'a [Item],
    /// The folders from the starting location down to the one shown: (id, name).
    pub trail: &'a [(String, String)],
    pub listing: Option<&'a Result<Vec<Item>, String>>,
    /// Save As: the existing file the user is being asked about replacing.
    pub replacing: bool,
    pub note: Option<&'a str>,
    /// The app's colour (GridCraft's is green): the left column in the light theme, the left
    /// column in the dark theme, and a selected row in the light theme.
    pub accent: [u32; 3],
}

/// GridCraft's colours, light or dark.
struct Tokens {
    dark: bool,
    bg: Color32,
    text: Color32,
    text_dim: Color32,
    accent: Color32,
    accent_soft: Color32,
    hover: Color32,
    nav_text: Color32,
    nav_hover: Color32,
}

impl Tokens {
    fn get(ctx: &egui::Context, accent: [u32; 3]) -> Tokens {
        let dark = ctx.style().visuals.dark_mode;
        let pick = |light: u32, dark_c: u32| {
            let c = if dark { dark_c } else { light };
            Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
        };
        Tokens {
            dark,
            bg: pick(0xFFFFFF, 0x333333),
            text: pick(0x242424, 0xEEEEEE),
            text_dim: pick(0x616161, 0xB0B0B0),
            accent: pick(accent[0], accent[1]),
            accent_soft: pick(accent[2], accent[1]),
            hover: pick(0xEBEBEB, 0x424242),
            nav_text: if dark { pick(0, 0xEEEEEE) } else { Color32::WHITE },
            nav_hover: if dark { Color32::from_white_alpha(17) } else { Color32::from_black_alpha(25) },
        }
    }
    fn cat(&self, light: u32, dark: u32) -> Color32 {
        let c = if self.dark { dark } else { light };
        Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
    }
}

/// GridCraft's icons on its 16-unit grid.
#[derive(Clone, Copy)]
enum Icon {
    Back,
    Folder,
    File,
    Clock,
    Plus,
}

fn paint_icon(p: &egui::Painter, rect: Rect, icon: Icon, c: Color32, t: &Tokens) {
    let side = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(side, side));
    let s = side / 16.0;
    let pt = |x: f32, y: f32| -> Pos2 { pos2(r.left() + x * s, r.top() + y * s) };
    let line = |pts: &[(f32, f32)], c: Color32, w: f32| {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| pt(*x, *y)).collect();
        p.add(PathShape::line(v, PathStroke::new(w * s.clamp(1.0, 2.0), c)));
    };
    match icon {
        Icon::Back => {
            p.circle_stroke(pt(8.0, 8.0), 6.5 * s, Stroke::new(1.0, c));
            line(&[(11.5, 8.0), (4.8, 8.0)], c, 1.2);
            line(&[(7.5, 5.3), (4.8, 8.0), (7.5, 10.7)], c, 1.2);
        }
        Icon::Folder => {
            let v: Vec<Pos2> = [(1.5, 4.0), (6.0, 4.0), (7.5, 5.5), (14.5, 5.5), (14.5, 13.0), (1.5, 13.0)].iter().map(|(x, y)| pt(*x, *y)).collect();
            p.add(PathShape::convex_polygon(v, t.cat(0x9C6C00, 0xF2C14E), Stroke::NONE));
        }
        Icon::File => {
            line(&[(9.5, 1.5), (3.5, 1.5), (3.5, 14.5), (12.5, 14.5), (12.5, 4.5), (9.5, 1.5), (9.5, 4.5), (12.5, 4.5)], c, 1.1);
        }
        Icon::Clock => {
            p.circle_stroke(pt(8.0, 8.0), 6.0 * s, Stroke::new(1.15 * s.clamp(1.0, 2.0), c));
            line(&[(8.0, 4.5), (8.0, 8.0), (10.8, 9.6)], t.cat(0x2F6FCF, 0x6EA8F7), 1.15);
        }
        Icon::Plus => {
            line(&[(8.0, 3.0), (8.0, 13.0)], c, 1.3);
            line(&[(3.0, 8.0), (13.0, 8.0)], c, 1.3);
        }
    }
}

/// One clickable row: icon, name and (when given) a dimmer line under it (GridCraft's `row`).
fn row(ui: &mut Ui, t: &Tokens, icon: Icon, name: &str, detail: &str, selected: bool) -> egui::Response {
    let height = if detail.is_empty() { 34.0 } else { 44.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 4.0, t.accent_soft);
    } else if resp.hovered() {
        p.rect_filled(rect, 4.0, t.hover);
    }
    paint_icon(p, Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(18.0, 18.0)), icon, t.text, t);
    let text = p.with_clip_rect(rect.intersect(p.clip_rect()));
    if detail.is_empty() {
        text.text(pos2(rect.left() + 38.0, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(13.5), t.text);
    } else {
        text.text(pos2(rect.left() + 38.0, rect.center().y - 8.0), Align2::LEFT_CENTER, name, FontId::proportional(13.5), t.text);
        text.text(pos2(rect.left() + 38.0, rect.center().y + 9.0), Align2::LEFT_CENTER, detail, FontId::proportional(11.5), t.text_dim);
    }
    resp
}

fn section(ui: &mut Ui, t: &Tokens, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).font(FontId::proportional(15.0)).strong().color(t.text));
    ui.add_space(8.0);
}

fn dim(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(text).font(FontId::proportional(12.5)).color(t.text_dim));
}

/// The navigation column's one row (GridCraft's `nav_item`).
fn nav_item(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(NAV_W, 38.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 0.0, t.nav_hover);
    }
    paint_icon(p, Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(17.0, 17.0)), icon, t.nav_text, t);
    p.text(pos2(rect.left() + 50.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), t.nav_text);
    resp
}

/// Draws the page over the whole window; returns what the user did. `name` is the Save As file
/// name being typed.
pub fn show(ctx: &egui::Context, view: &View, name: &mut String) -> Vec<Action> {
    let t = Tokens::get(ctx, view.accent);
    let mut out = Vec::new();
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("android_file_page")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        ui.set_min_size(screen.size());
        ui.set_max_size(screen.size());
        ui.painter().rect_filled(screen, 0.0, t.bg);
        let nav = Rect::from_min_size(screen.min, vec2(NAV_W, screen.height()));
        ui.painter().rect_filled(nav, 0.0, t.accent);
        // Left column: Back, as on GridCraft's File pages.
        ui.scope_builder(egui::UiBuilder::new().max_rect(nav.shrink2(vec2(0.0, 10.0))), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if nav_item(ui, &t, Icon::Back, "Back").clicked() {
                out.push(Action::Cancel);
            }
        });
        let content = Rect::from_min_max(pos2(nav.right() + 44.0, screen.top() + 26.0), pos2(screen.right() - 32.0, screen.bottom() - 16.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            ui.set_max_width(CONTENT_W.min(content.width()));
            ui.label(RichText::new(view.title).font(FontId::proportional(28.0)).color(t.text));
            ui.add_space(18.0);
            if view.saving {
                ui.horizontal(|ui| {
                    let label = ui.label("File name");
                    let resp = ui.add(egui::TextEdit::singleline(name).desired_width(360.0)).labelled_by(label.id);
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let ready = !view.trail.is_empty() && !name.trim().is_empty();
                    if (ui.add_enabled(ready, egui::Button::new("Save")).clicked() || enter) && ready {
                        out.push(Action::Save);
                    }
                });
                if view.replacing {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        dim(ui, &t, &format!("{} already exists. Replace it?", name.trim()));
                        if ui.button("Replace").clicked() {
                            out.push(Action::Replace);
                        }
                    });
                }
                ui.add_space(16.0);
            }
            if view.choosing_folder {
                let ready = !view.trail.is_empty();
                if ui.add_enabled(ready, egui::Button::new("Choose this folder")).clicked() {
                    out.push(Action::ChooseFolder);
                }
                ui.add_space(16.0);
            }
            if let Some(note) = view.note {
                ui.colored_label(ui.visuals().warn_fg_color, note);
                ui.add_space(8.0);
            }
            let height = (ui.available_height() - 8.0).max(120.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(SIDE_W);
                    egui::ScrollArea::vertical().id_salt("android_file_places").max_height(height).show(ui, |ui| {
                        section(ui, &t, "Locations");
                        let at = view.trail.first().map(|(id, _)| id.as_str());
                        for p in view.places {
                            if row(ui, &t, Icon::Folder, &p.name, "", at == Some(p.id.as_str())).clicked() {
                                out.push(Action::Go(p.clone()));
                            }
                        }
                        if row(ui, &t, Icon::Plus, "Add a location…", "", false).clicked() {
                            out.push(Action::AddPlace);
                        }
                        if !view.recent.is_empty() {
                            ui.add_space(14.0);
                            section(ui, &t, "Recent folders");
                            for r in view.recent {
                                if row(ui, &t, Icon::Clock, &r.name, &r.detail, false).clicked() {
                                    out.push(Action::Go(r.clone()));
                                }
                            }
                        }
                    });
                });
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.set_width((CONTENT_W - SIDE_W - 24.0).min(ui.available_width()).max(200.0));
                    if view.trail.is_empty() {
                        dim(ui, &t, &format!("Choose Add a location… and pick a folder {} may use, such as My files.", view.app));
                        return;
                    }
                    // Where we are: each folder on the way down can be clicked to go back up to it.
                    ui.horizontal_wrapped(|ui| {
                        let last = view.trail.len() - 1;
                        for (i, (_, folder)) in view.trail.iter().enumerate() {
                            if i > 0 {
                                ui.label("›");
                            }
                            if i == last {
                                ui.label(RichText::new(folder.as_str()).strong());
                            } else if ui.link(folder.as_str()).clicked() {
                                out.push(Action::Up(i));
                            }
                        }
                    });
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().id_salt("android_file_listing").max_height(height).show(ui, |ui| match view.listing {
                        Some(Ok(items)) if items.is_empty() => dim(ui, &t, "Nothing here to open."),
                        Some(Ok(items)) => {
                            for item in items {
                                let icon = if item.folder { Icon::Folder } else { Icon::File };
                                let selected = view.saving && !item.folder && name.trim() == item.name;
                                if row(ui, &t, icon, &item.name, "", selected).clicked() {
                                    out.push(Action::Pick(item.clone()));
                                }
                            }
                        }
                        Some(Err(e)) => dim(ui, &t, e),
                        None => {}
                    });
                });
            });
        });
    });
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        out.push(Action::Cancel);
    }
    out
}
