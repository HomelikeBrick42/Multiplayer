use cgmath::Vector2;
use client::{Circle, Client, ClientToServerMessage, ServerToClientMessage};
use eframe::{egui, egui_wgpu::Callback};
use renderer::{create_render_state, GpuCamera, GpuCircle, RenderCallback};
use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
};

pub mod client;
pub mod renderer;

struct Camera {
    position: Vector2<f32>,
    zoom: f32,
}

pub struct App {
    camera: Camera,
    circle: Circle,
    circles: HashMap<SocketAddr, Circle>,
    client: Client,
    _runtime: tokio::runtime::Runtime,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, host: bool) -> Self {
        create_render_state(cc);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let app = Self {
            camera: Camera {
                position: cgmath::vec2(0.0, 0.0),
                zoom: 1.0,
            },
            circle: Circle {
                position: cgmath::vec2(0.0, 0.0),
                color: cgmath::vec3(0.0, 0.0, 1.0),
                radius: 0.5,
            },
            circles: HashMap::new(),
            client: runtime.block_on(async {
                let [addr] = "127.0.0.1:1234"
                    .to_socket_addrs()
                    .unwrap()
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap();
                if host {
                    Client::create_local(addr).await.unwrap()
                } else {
                    Client::connect(addr).await.unwrap()
                }
            }),
            _runtime: runtime,
        };
        app.client
            .send_message(ClientToServerMessage::PlayerChanged(app.circle))
            .unwrap();
        app
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        while let Some(message) = self.client.get_message() {
            match message.unwrap() {
                ServerToClientMessage::ClientConnected(addr) => {
                    let new = self
                        .circles
                        .insert(
                            addr,
                            Circle {
                                position: cgmath::vec2(0.0, 0.0),
                                color: cgmath::vec3(1.0, 0.0, 1.0),
                                radius: 0.5,
                            },
                        )
                        .is_none();
                    assert!(new);
                }
                ServerToClientMessage::ClientDisconnected(addr) => {
                    let exists = self.circles.remove(&addr).is_some();
                    assert!(exists);
                }
                ServerToClientMessage::Ping => {
                    self.client
                        .send_message(ClientToServerMessage::Ping)
                        .unwrap();
                }
                ServerToClientMessage::PlayerChanged(addr, circle) => {
                    *self.circles.get_mut(&addr).unwrap() = circle;
                }
            }
        }

        egui::Window::new("Circle Settings").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Color: ");
                if ui
                    .color_edit_button_rgb(self.circle.color.as_mut())
                    .changed()
                {
                    self.client
                        .send_message(ClientToServerMessage::PlayerChanged(self.circle))
                        .unwrap();
                }
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
                let aspect = rect.width() / rect.height();

                if response.dragged_by(egui::PointerButton::Secondary) {
                    let delta = response.drag_delta();
                    self.camera.position.x -=
                        delta.x / self.camera.zoom / rect.width() * 2.0 * aspect;
                    self.camera.position.y += delta.y / self.camera.zoom / rect.height() * 2.0;
                }

                if response.clicked_by(egui::PointerButton::Primary) {
                    let interact_pointer_pos = response.interact_pointer_pos().unwrap();
                    let mouse_position = ((interact_pointer_pos - rect.left_top()) / rect.size()
                        * 2.0
                        - egui::vec2(1.0, 1.0))
                        * egui::vec2(1.0, -1.0);
                    let world_position = Vector2 {
                        x: mouse_position.x * aspect / self.camera.zoom + self.camera.position.x,
                        y: mouse_position.y / self.camera.zoom + self.camera.position.y,
                    };

                    self.circle.position = world_position;
                    self.client
                        .send_message(ClientToServerMessage::PlayerChanged(self.circle))
                        .unwrap();
                }

                if response.hovered() {
                    ctx.input(|input| match input.scroll_delta.y.total_cmp(&0.0) {
                        std::cmp::Ordering::Less => self.camera.zoom *= 0.9,
                        std::cmp::Ordering::Greater => self.camera.zoom /= 0.9,
                        _ => {}
                    });
                }

                ui.painter().add(Callback::new_paint_callback(
                    rect,
                    RenderCallback {
                        camera: GpuCamera {
                            position: self.camera.position,
                            aspect,
                            zoom: self.camera.zoom,
                        },
                        circles: self
                            .circles
                            .values()
                            .map(
                                |&Circle {
                                     position,
                                     color,
                                     radius,
                                 }| GpuCircle {
                                    position,
                                    color,
                                    radius,
                                },
                            )
                            .collect(),
                    },
                ));
            });

        ctx.request_repaint();
    }
}
