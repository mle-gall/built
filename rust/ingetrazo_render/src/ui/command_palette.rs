//! Command search shared by the model viewport and the detached sheet canvas.
use super::*;

pub fn fit_box(cursor: Pos2, size: Vec2, field_height: f32, work: Rect, screen: Rect) -> Rect {
    fn axis(pos: f32, size: f32, lo: f32, hi: f32, outer_lo: f32, outer_hi: f32) -> f32 {
        let (lo, hi) = if size <= hi - lo {
            (lo, hi)
        } else {
            (outer_lo, outer_hi)
        };
        pos.clamp(lo, (hi - size).max(lo))
    }
    let size = size.min(screen.size());
    let x = axis(
        cursor.x - size.x / 2.,
        size.x,
        work.left(),
        work.right(),
        screen.left(),
        screen.right(),
    );
    let y = axis(
        cursor.y - field_height / 2.,
        size.y,
        work.top(),
        work.bottom(),
        screen.top(),
        screen.bottom(),
    );
    Rect::from_min_size(pos2(x, y), size)
}

impl Ui {
    pub fn start_commands(&mut self, text: String, scope: String) {
        self.cmd_open = true;
        self.cmd_results.clear();
        self.cmd_extra.clear();
        self.cmd_anchor = self.ctx.pointer_latest_pos();
        self.cmd_at = 0;
        self.cmd_focus = true;
        self.actions.push(format!("cmdscope:{scope}"));
        self.actions.push(format!("cmdq:{text}"));
        self.cmd_scope = scope;
        self.cmd_query = text;
    }

    pub fn command_details(&mut self, details: Vec<(String, bool, bool)>) {
        self.cmd_extra = details;
    }

    /// ⌘K / F3: every command of the window by name, at the top of the model.
    pub(super) fn command_palette(&mut self, ctx: &egui::Context, screen: Rect) {
        if !self.cmd_open {
            return;
        }
        let mut run: Option<usize> = None;
        let mut changed = false;
        let mut change_shortcut = None;
        let bounds = if self.cmd_canvas {
            screen
        } else {
            let work = Rect::from_min_max(
                pos2(GUT + 2. * CONTROL_H, TOP_H),
                pos2(self.free_right(ctx, screen), screen.bottom() - BOT_H),
            );
            fit_box(
                self.cmd_anchor.unwrap_or(work.center()),
                vec2(560., 310.),
                34.,
                work,
                screen,
            )
        };
        egui::Area::new(egui::Id::new("commands"))
            .fixed_pos(bounds.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                glass(Surface::Raised)
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        let width = (bounds.width() - 18.).max(1.);
                        ui.set_width(width);
                        if !self.cmd_scope.is_empty() {
                            ui.label(
                                egui::RichText::new(&self.cmd_scope)
                                    .font(meta())
                                    .color(FAINT),
                            );
                        }
                        let r = ui.add(
                            kit::field(&mut self.cmd_query)
                                .hint_text(egui::RichText::new("Search commands").color(FAINT))
                                .font(body())
                                .margin(Margin::symmetric(10, 7))
                                .desired_width((width - 24.).max(1.))
                                .clip_text(true),
                        );
                        if self.cmd_focus {
                            r.request_focus();
                            self.cmd_focus = false;
                        }
                        if r.changed() {
                            changed = true;
                            self.cmd_at = 0;
                        }
                        ui.add_space(SPACE_S);
                        ui.spacing_mut().item_spacing = vec2(0.0, 1.0);
                        egui::ScrollArea::vertical()
                            .max_height((CONTROL_H + 1.0) * 10.0)
                            .show(ui, |ui| {
                                for (i, (label, path, key, enabled)) in
                                    self.cmd_results.iter().enumerate()
                                {
                                    let detail = self.cmd_extra.get(i);
                                    let checked = detail.is_some_and(|d| d.1);
                                    let text = if checked {
                                        format!("✓ {label}")
                                    } else {
                                        label.clone()
                                    };
                                    let row = command_row(
                                        ui,
                                        &text,
                                        path,
                                        key,
                                        *enabled,
                                        self.cmd_at == i,
                                    );
                                    let row = row.on_hover_ui(|ui| {
                                        ui.set_max_width(300.);
                                        ui.label(
                                            egui::RichText::new(label)
                                                .font(title_font())
                                                .color(INK),
                                        );
                                        if let Some((tip, _, _)) =
                                            detail.filter(|d| !d.0.is_empty())
                                        {
                                            ui.label(tip);
                                        }
                                        if !key.is_empty() {
                                            ui.label(
                                                egui::RichText::new(key).font(meta()).color(FAINT),
                                            );
                                        }
                                    });
                                    if detail.is_some_and(|d| d.2) {
                                        row.context_menu(|ui| {
                                            if quiet(ui, "Change shortcut…").clicked() {
                                                change_shortcut = Some(i);
                                                ui.close_menu();
                                            }
                                        });
                                    }
                                    if self.cmd_scroll && i == self.cmd_at {
                                        row.scroll_to_me(None);
                                    }
                                    if *enabled && row.clicked() {
                                        run = Some(i);
                                    }
                                    if *enabled
                                        && row.hovered()
                                        && ui.input(|i| i.pointer.delta() != Vec2::ZERO)
                                    {
                                        self.cmd_at = i;
                                    }
                                }
                            });
                        self.cmd_scroll = false;
                        if self.cmd_results.is_empty() && !self.cmd_query.is_empty() {
                            ui.label(
                                egui::RichText::new("No command matches")
                                    .font(body())
                                    .color(FAINT),
                            );
                        }
                    });
            });
        if changed {
            self.actions.push(format!("cmdq:{}", self.cmd_query));
        }
        if let Some(i) = change_shortcut {
            self.cmd_open = false;
            self.actions.push(format!("cmdshortcut:{i}"));
        }
        if let Some(i) = run {
            self.cmd_open = false;
            self.actions.push(format!("cmdrun:{i}"));
        }
    }
}

use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyBytes};
use std::sync::OnceLock;

type BoxTuple = (f32, f32, f32, f32);
type CommandRow = (String, String, String, bool);

/// The detached Composer has a Qt document canvas. Render the same egui
/// control into a small image that the document canvas can composite.
#[pyclass(unsendable)]
pub struct CommandCanvas {
    ui: Ui,
    width: u32,
    height: u32,
    scale: f32,
    frame: wgpu::Texture,
    staging: wgpu::Buffer,
    stride: u32,
    gpu: &'static (wgpu::Device, wgpu::Queue),
}

#[pymethods]
impl CommandCanvas {
    #[new]
    fn new(width: u32, height: u32, scale: f32, fonts: &str) -> PyResult<Self> {
        static GPU: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
        if GPU.get().is_none() {
            let adapter =
                pollster::block_on(crate::backend::instance().request_adapter(&Default::default()))
                    .ok_or_else(|| {
                        PyRuntimeError::new_err(
                            "Command search could not acquire a graphics device",
                        )
                    })?;
            let gpu = pollster::block_on(adapter.request_device(&Default::default(), None))
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            let _ = GPU.set(gpu);
        }
        let gpu = GPU.get().unwrap();
        let width = (width as f32 * scale).round().max(1.) as u32;
        let height = (height as f32 * scale).round().max(1.) as u32;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let frame = crate::texture(&gpu.0, width, height, format);
        let stride = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = gpu.0.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Command search canvas"),
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut ui = Ui::new(&gpu.0, format);
        let dir = std::path::Path::new(fonts);
        let read = |name| std::fs::read(dir.join(name)).ok();
        if let Some(regular) = read("Inter-Regular.ttf") {
            ui.set_fonts(
                regular,
                read("Inter-Medium.ttf"),
                read("Inter-SemiBold.ttf"),
            );
        }
        ui.cmd_canvas = true;
        Ok(Self {
            ui,
            width,
            height,
            scale,
            frame,
            staging,
            stride,
            gpu,
        })
    }

    #[staticmethod]
    fn geometry(
        cursor: (f32, f32),
        size: (f32, f32),
        edit_height: f32,
        work: BoxTuple,
        screen: BoxTuple,
    ) -> (i32, i32, i32, i32) {
        let rect = |v: BoxTuple| Rect::from_min_size(pos2(v.0, v.1), vec2(v.2, v.3));
        let r = fit_box(
            pos2(cursor.0, cursor.1),
            vec2(size.0, size.1),
            edit_height,
            rect(work),
            rect(screen),
        );
        (
            r.left() as i32,
            r.top() as i32,
            r.width() as i32,
            r.height() as i32,
        )
    }

    fn open(&mut self, text: String, scope: String) {
        self.ui.start_commands(text, scope);
    }
    fn ui_set_command_results(&mut self, rows: Vec<CommandRow>) {
        self.ui.set_command_results(rows);
    }
    fn ui_set_command_details(&mut self, rows: Vec<(String, bool, bool)>) {
        self.ui.command_details(rows);
    }
    fn actions(&mut self) -> Vec<String> {
        self.ui.take_actions()
    }
    fn rows(&self) -> Vec<CommandRow> {
        self.ui.cmd_results.clone()
    }
    fn details(&self) -> Vec<(String, bool, bool)> {
        self.ui.cmd_extra.clone()
    }
    fn state(&self) -> (bool, String, String, usize) {
        (
            self.ui.cmd_open,
            self.ui.cmd_query.clone(),
            self.ui.cmd_scope.clone(),
            self.ui.cmd_at,
        )
    }
    fn key(&mut self, name: &str, pressed: bool, mods: u8, text: &str) {
        self.ui.key(name, pressed, mods);
        if pressed && !text.is_empty() {
            self.ui.text(text.to_owned());
        }
    }
    fn pointer(&mut self, kind: &str, x: f32, y: f32, button: u8, mods: u8) {
        self.ui.pointer(kind, x, y, button, mods);
    }
    fn wheel(&mut self, x: f32, y: f32, dx: f32, dy: f32) {
        self.ui.wheel(x, y, dx, dy);
    }

    fn frame<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let (device, queue) = self.gpu;
        let mut encoder = device.create_command_encoder(&Default::default());
        let target = self.frame.create_view(&Default::default());
        let background = SHEET;
        let channel = |n: u8| (f64::from(n) / 255.).powf(2.2);
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Command canvas backdrop"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: channel(background.r()),
                        g: channel(background.g()),
                        b: channel(background.b()),
                        a: 1.,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        self.ui.paint(
            device,
            queue,
            &mut encoder,
            &self.frame,
            &target,
            [self.width, self.height],
            self.scale,
        );
        encoder.copy_texture_to_buffer(
            self.frame.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: None,
                },
            },
            self.frame.size(),
        );
        queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::channel();
        let slice = self.staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = send.send(res);
        });
        device.poll(wgpu::Maintain::Wait);
        recv.recv()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let data = slice.get_mapped_range();
        let pixels = PyBytes::new_with(py, (4 * self.width * self.height) as usize, |out| {
            let w = self.width as usize * 4;
            for (dest, src) in out
                .chunks_exact_mut(w)
                .zip(data.chunks_exact(self.stride as usize))
            {
                dest.copy_from_slice(&src[..w]);
            }
            Ok(())
        });
        drop(data);
        self.staging.unmap();
        pixels
    }
}
