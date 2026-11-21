//! Brush Studio: a desktop and web editor for the Brush engine.
//!
//! One window: an orbit viewport rendered by the deterministic software rasteriser, a tool shelf (primitives, splat
//! spray/erase, bake), and a journal + measure panel. Every edit is an engine op, so the journal stays replayable
//! and the hash stays comparable with every other target.

use brush_engine::{Actor, Applied, Document};
use brush_mesh::math::{norm, Vec3};
use brush_mesh::{Measure, Mesh, Op};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui};

/// The empty document starts as a studio backdrop sphere so spray painting has something to land on.
const START_RADIUS: f64 = 1.0;
const MAX_VIEW: (u32, u32) = (1400, 900);

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Spray,
    Erase,
}

struct Orbit {
    yaw: f32,
    pitch: f32,
    dist: f32,
    target: Vec3,
}

impl Orbit {
    fn eye(&self) -> Vec3 {
        let (sy, cy) = (libm::sinf(self.yaw), libm::cosf(self.yaw));
        let (sp, cp) = (libm::sinf(self.pitch), libm::cosf(self.pitch));
        [
            self.target[0] + self.dist * sy * cp,
            self.target[1] + self.dist * sp,
            self.target[2] + self.dist * cy * cp,
        ]
    }

    fn camera(&self) -> brush_render::Camera {
        brush_render::Camera {
            eye: self.eye().map(|v| v as f64),
            target: self.target.map(|v| v as f64),
            fov_deg: 60.0,
        }
    }
}

pub struct StudioApp {
    doc: Document,
    orbit: Orbit,
    tool: Tool,
    spray_color: [u8; 3],
    spray_radius: f64,
    spray_size: f64,
    spray_flow: f64,
    spray_opacity: f64,
    spray_drip: f64,
    erase_radius: f64,
    stroke_seed: u32,
    stroke_dabs: u32,
    tex: Option<TextureHandle>,
    tex_size: (u32, u32),
    view_dirty: bool,
    log: Vec<String>,
    status: String,
    measure: Option<Measure>,
    save_name: String,
}

impl StudioApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            doc: Document::new(Mesh::default()),
            orbit: Orbit {
                yaw: 0.7,
                pitch: 0.4,
                dist: 3.2,
                target: [0.0, 0.0, 0.0],
            },
            tool: Tool::Spray,
            spray_color: [185, 28, 28],
            spray_radius: 0.4,
            spray_size: 0.06,
            spray_flow: 0.5,
            spray_opacity: 1.0,
            spray_drip: 0.0,
            erase_radius: 0.5,
            stroke_seed: 1,
            stroke_dabs: 0,
            tex: None,
            tex_size: (0, 0),
            view_dirty: true,
            log: Vec::new(),
            status: "Spray with the left mouse button; orbit with the right (or Shift-drag)."
                .into(),
            measure: None,
            save_name: "studio.ply".into(),
        };
        app.apply_json(serde_json::json!({
            "op": "addSphere",
            "center": [0.0, 0.0, 0.0],
            "radii": [START_RADIUS, START_RADIUS, START_RADIUS],
            "segments": 48,
            "rings": 24,
        }));
        app
    }

    fn actor() -> Actor {
        Actor {
            id: "studio".into(),
            kind: None,
        }
    }

    /// Builds an op from JSON (the same wire format every other target uses) and applies it.
    fn apply_json(&mut self, value: serde_json::Value) -> Option<Applied> {
        let op: Op = match serde_json::from_value(value) {
            Ok(op) => op,
            Err(e) => {
                self.status = format!("bad op: {e}");
                return None;
            }
        };
        let name = op.name();
        match self.doc.apply(op, Self::actor()) {
            Ok(applied) => {
                self.log
                    .push(format!("{name} (+{})", applied.outcome.count));
                self.view_dirty = true;
                self.measure = None;
                Some(applied)
            }
            Err(e) => {
                self.status = format!("op rejected: {e}");
                None
            }
        }
    }

    fn spray_op(&self, center: Vec3, view: Vec3) -> serde_json::Value {
        serde_json::json!({
            "op": "splatPaint",
            "dabs": [{ "center": center, "pressure": 1.0 }],
            "radius": self.spray_radius,
            "size": self.spray_size,
            "color": self.spray_color,
            "opacity": self.spray_opacity,
            "flow": self.spray_flow,
            "variation": 0.25,
            "colorNoise": 0.15,
            "drip": self.spray_drip,
            "seed": self.stroke_seed.wrapping_add(self.stroke_dabs),
            "view": view,
            "align": true,
        })
    }

    /// A ray through the pointer, in the same projection the rasteriser uses.
    fn pointer_ray(&self, rect: Rect, pos: Pos2) -> Option<(Vec3, Vec3)> {
        let w = rect.width().max(1.0);
        let h = rect.height().max(1.0);
        let nx = (pos.x - rect.min.x) / w * 2.0 - 1.0;
        let ny = -((pos.y - rect.min.y) / h * 2.0 - 1.0);
        let eye = self.orbit.eye();
        let fwd = norm([
            self.orbit.target[0] - eye[0],
            self.orbit.target[1] - eye[1],
            self.orbit.target[2] - eye[2],
        ]);
        // Same basis as brush_render::render: right = norm(cross(fwd, +y)), up = cross(right, fwd).
        let right = norm([-fwd[2], 0.0, fwd[0]]);
        let up = [
            right[1] * fwd[2] - right[2] * fwd[1],
            right[2] * fwd[0] - right[0] * fwd[2],
            right[0] * fwd[1] - right[1] * fwd[0],
        ];
        let f = 1.0 / libm::tanf(60.0_f32.to_radians() * 0.5);
        let aspect = w / h;
        let dir = norm([
            right[0] * nx * aspect / f + up[0] * ny / f + fwd[0],
            right[1] * nx * aspect / f + up[1] * ny / f + fwd[1],
            right[2] * nx * aspect / f + up[2] * ny / f + fwd[2],
        ]);
        Some((eye, dir))
    }

    fn paint_at(&mut self, rect: Rect, pos: Pos2) {
        let Some((eye, dir)) = self.pointer_ray(rect, pos) else {
            return;
        };
        let Some(hit) = self.doc.raycast(eye, dir) else {
            return;
        };
        match self.tool {
            Tool::Spray => {
                if self
                    .apply_json(self.spray_op(hit.point, hit.normal))
                    .is_some()
                {
                    self.stroke_dabs += 1;
                }
            }
            Tool::Erase => {
                self.apply_json(serde_json::json!({
                    "op": "splatErase",
                    "dabs": [{ "center": hit.point, "pressure": 1.0 }],
                    "radius": self.erase_radius,
                }));
                self.stroke_dabs += 1;
            }
        }
    }

    fn tool_panel(&mut self, ui: &mut Ui) {
        ui.heading("Create");
        ui.horizontal(|ui| {
            if ui.button("Box").clicked() {
                self.apply_json(serde_json::json!({
                    "op": "addBox",
                    "box": { "center": [0.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0] },
                }));
            }
            if ui.button("Sphere").clicked() {
                self.apply_json(serde_json::json!({
                    "op": "addSphere",
                    "center": [0.0, 0.0, 0.0],
                    "radii": [0.75, 0.75, 0.75],
                    "segments": 48,
                    "rings": 24,
                }));
            }
            if ui.button("Lathe").clicked() {
                self.apply_json(serde_json::json!({
                    "op": "addLathe",
                    "base": [0.0, -0.8, 0.0],
                    "profile": [[0.05, 0.0], [0.5, 0.05], [0.42, 0.5], [0.55, 1.0], [0.3, 1.5], [0.02, 1.6]],
                }));
            }
        });
        ui.separator();
        ui.heading("Splat paint");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tool, Tool::Spray, "Spray");
            ui.selectable_value(&mut self.tool, Tool::Erase, "Erase");
        });
        ui.add(egui::Slider::new(&mut self.spray_radius, 0.05..=2.0).text("radius"));
        ui.add(egui::Slider::new(&mut self.spray_size, 0.01..=0.3).text("splat size"));
        ui.add(egui::Slider::new(&mut self.spray_flow, 0.05..=1.0).text("flow"));
        ui.add(egui::Slider::new(&mut self.spray_opacity, 0.05..=1.0).text("opacity"));
        ui.add(egui::Slider::new(&mut self.spray_drip, 0.0..=1.0).text("drip"));
        ui.add(egui::Slider::new(&mut self.erase_radius, 0.05..=2.0).text("erase radius"));
        ui.horizontal(|ui| {
            ui.label("colour");
            let mut c = Color32::from_rgb(
                self.spray_color[0],
                self.spray_color[1],
                self.spray_color[2],
            );
            if ui.color_edit_button_srgba(&mut c).changed() {
                self.spray_color = [c.r(), c.g(), c.b()];
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Bake splats to vertices").clicked() {
                self.apply_json(
                    serde_json::json!({ "op": "splatBake", "strength": 1.0, "clear": true }),
                );
            }
            if ui.button("Clear splats").clicked() {
                self.apply_json(serde_json::json!({
                    "op": "splatErase",
                    "dabs": [{ "center": [0.0, 0.0, 0.0], "pressure": 1.0 }],
                    "radius": 1.0e9,
                }));
            }
        });
        ui.separator();
        ui.heading("History");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.doc.can_undo(), egui::Button::new("Undo"))
                .clicked()
                && self.doc.undo()
            {
                self.log.push("undo".into());
                self.view_dirty = true;
                self.measure = None;
            }
            if ui
                .add_enabled(self.doc.can_redo(), egui::Button::new("Redo"))
                .clicked()
                && self.doc.redo()
            {
                self.log.push("redo".into());
                self.view_dirty = true;
                self.measure = None;
            }
        });
        ui.separator();
        ui.heading("File");
        ui.horizontal(|ui| {
            ui.label("name");
            ui.add(egui::TextEdit::singleline(&mut self.save_name).desired_width(90.0));
        });
        ui.horizontal(|ui| {
            if ui.button("Save PLY").clicked() {
                self.save_ply();
            }
            if ui.button("Open PLY").clicked() {
                self.open_ply();
            }
        });
        if ui.button("Reset view").clicked() {
            self.orbit = Orbit {
                yaw: 0.7,
                pitch: 0.4,
                dist: 3.2,
                target: [0.0, 0.0, 0.0],
            };
            self.view_dirty = true;
        }
    }

    fn side_panel(&mut self, ui: &mut Ui) {
        let mesh = self.doc.mesh();
        ui.heading("Model");
        ui.label(format!("vertices  {}", mesh.vertex_count()));
        ui.label(format!("faces     {}", mesh.face_count()));
        ui.label(format!("splats    {}", mesh.splats.len()));
        ui.label(format!("hash      {}", self.doc.hash()));
        if ui.button("Measure").clicked() {
            self.measure = Some(mesh.measure());
        }
        if let Some(m) = &self.measure {
            ui.label(format!("volume    {:.4}", m.volume));
            ui.label(format!("area      {:.4}", m.surface_area));
            ui.label(format!(
                "centroid  {:.3} {:.3} {:.3}",
                m.centroid[0], m.centroid[1], m.centroid[2]
            ));
            ui.label(format!("closed    {}", m.closed));
        }
        ui.separator();
        ui.heading("Journal");
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, line) in self.log.iter().enumerate().rev() {
                    ui.label(format!("{:>3}  {line}", i + 1));
                }
            });
    }

    fn viewport(&mut self, ui: &mut Ui) {
        let avail = ui.available_size();
        // Render at the window size, capped so the software rasteriser stays responsive on huge monitors.
        let w = (avail.x as u32).clamp(16, MAX_VIEW.0);
        let h = (avail.y as u32).clamp(16, MAX_VIEW.1);
        let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
        let view_rect = rect;

        if self.view_dirty || self.tex_size != (w, h) {
            let rgba = brush_render::render(self.doc.mesh(), w, h, &self.orbit.camera());
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            match &mut self.tex {
                Some(tex) => tex.set(img, TextureOptions::NEAREST),
                None => {
                    self.tex = Some(ui.ctx().load_texture(
                        "viewport",
                        img,
                        TextureOptions::NEAREST,
                    ));
                }
            }
            self.tex_size = (w, h);
            self.view_dirty = false;
        }
        if let Some(tex) = &self.tex {
            ui.painter().image(
                tex.id(),
                view_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // Input: left button sprays/erases, right button or shift-left orbits, scroll zooms, middle drag pans.
        if resp.drag_started_by(egui::PointerButton::Primary) {
            self.stroke_seed = self.stroke_seed.wrapping_mul(2_269_547_733).wrapping_add(1);
            self.stroke_dabs = 0;
        }
        let shift = ui.ctx().input(|i| i.modifiers.shift);
        if resp.dragged_by(egui::PointerButton::Secondary)
            || (resp.dragged_by(egui::PointerButton::Primary) && shift)
        {
            let d = resp.drag_delta();
            self.orbit.yaw -= d.x * 0.008;
            self.orbit.pitch = (self.orbit.pitch + d.y * 0.008).clamp(-1.5, 1.5);
            self.view_dirty = true;
        } else if resp.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                self.paint_at(view_rect, pos);
            }
        }
        if resp.clicked_by(egui::PointerButton::Primary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                self.paint_at(view_rect, pos);
            }
        }
        if resp.dragged_by(egui::PointerButton::Middle) {
            let d = resp.drag_delta();
            let eye = self.orbit.eye();
            let fwd = norm([
                self.orbit.target[0] - eye[0],
                self.orbit.target[1] - eye[1],
                self.orbit.target[2] - eye[2],
            ]);
            let right = norm([-fwd[2], 0.0, fwd[0]]);
            let up = [
                right[1] * fwd[2] - right[2] * fwd[1],
                right[2] * fwd[0] - right[0] * fwd[2],
                right[0] * fwd[1] - right[1] * fwd[0],
            ];
            let k = self.orbit.dist * 0.0016;
            for i in 0..3 {
                self.orbit.target[i] += (-d.x * right[i] + d.y * up[i]) * k;
            }
            self.view_dirty = true;
        }
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 && resp.hovered() {
            self.orbit.dist = (self.orbit.dist * libm::expf(-scroll * 0.001)).clamp(0.2, 60.0);
            self.view_dirty = true;
        }
        if self.view_dirty {
            ui.ctx().request_repaint();
        }
    }

    fn save_ply(&mut self) {
        let bytes = brush_io::write_ply(
            self.doc.mesh(),
            brush_io::PlyOptions {
                geometry_only: false,
            },
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
            let name = if self.save_name.ends_with(".ply") {
                self.save_name.clone()
            } else {
                format!("{}.ply", self.save_name)
            };
            if let Some(path) = rfd::FileDialog::new().set_file_name(&name).save_file() {
                match std::fs::write(&path, &bytes) {
                    Ok(()) => {
                        self.status = format!("saved {} ({} bytes)", path.display(), bytes.len())
                    }
                    Err(e) => self.status = format!("save failed: {e}"),
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            download(&self.save_name, &bytes);
            self.status = format!("downloaded {} ({} bytes)", self.save_name, bytes.len());
        }
    }

    fn open_ply(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PLY", &["ply"])
            .pick_file()
        {
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| {
                    brush_io::read_ply(&b)
                        .map(|(m, _)| m)
                        .map_err(|e| e.to_string())
                }) {
                Ok(mesh) => {
                    self.doc = Document::new(mesh);
                    self.log.clear();
                    self.view_dirty = true;
                    self.measure = None;
                    self.status = format!("opened {}", path.display());
                }
                Err(e) => self.status = format!("open failed: {e}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.status =
                "open is a desktop feature; the web build starts from the backdrop sphere".into();
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Brush Studio");
                ui.separator();
                ui.label(&self.status);
            });
        });
        egui::SidePanel::left("tools")
            .default_width(210.0)
            .show(ctx, |ui| self.tool_panel(ui));
        egui::SidePanel::right("side")
            .default_width(230.0)
            .show(ctx, |ui| self.side_panel(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.viewport(ui));
    }
}

/// Triggers a browser download for `bytes` (web build only).
#[cfg(target_arch = "wasm32")]
fn download(name: &str, bytes: &[u8]) {
    use eframe::wasm_bindgen::JsCast;
    let name = if name.ends_with(".ply") {
        name.to_string()
    } else {
        format!("{name}.ply")
    };
    let Some(window) = eframe::web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&array);
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    if let Ok(a) = document.create_element("a") {
        let a: web_sys::HtmlAnchorElement = a.unchecked_into();
        a.set_href(&url);
        a.set_download(&name);
        a.click();
        let _ = web_sys::Url::revoke_object_url(&url);
    }
}
