use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Theme, Ui, Vec2};
use mesh_core::{AppScreen, RuntimePhase, UiCommand, UiSnapshot};
use mesh_node::NodeHandle;

pub struct MeshApp {
    handle: NodeHandle,
    snapshots: tokio::sync::watch::Receiver<UiSnapshot>,
    display_name: String,
    invitation_input: String,
    shutdown_sent: bool,
}

impl MeshApp {
    pub fn new(cc: &eframe::CreationContext<'_>, handle: NodeHandle) -> Self {
        style_visuals(&cc.egui_ctx);
        let snapshots = handle.subscribe_snapshots();
        let display_name = snapshots.borrow().local.display_name.clone();
        Self {
            handle,
            snapshots,
            display_name,
            invitation_input: String::new(),
            shutdown_sent: false,
        }
    }

    fn snapshot(&self) -> UiSnapshot {
        self.snapshots.borrow().clone()
    }

    fn send(&mut self, command: UiCommand) {
        if let Err(error) = self.handle.try_send(command) {
            tracing::warn!(%error, "failed to send UI command");
        }
    }

    fn request_shutdown(&mut self) {
        if self.shutdown_sent {
            return;
        }
        self.shutdown_sent = true;
        let _ = self.handle.request_shutdown();
    }
}

impl eframe::App for MeshApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        if ctx.input(|input| input.viewport().close_requested()) {
            self.request_shutdown();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let snapshot = self.snapshot();

        egui::Panel::top("title_bar").show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(RichText::new("Mesh").strong());
                ui.label(RichText::new("Direct PC compute").weak());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    phase_badge(ui, snapshot.phase);
                });
            });
            ui.add_space(4.0);
            ui.separator();
        });

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&snapshot.status_message).weak());
            });
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(12.0);
            match snapshot.screen {
                AppScreen::FirstRun => self.draw_first_run(ui),
                AppScreen::Enroll => self.draw_enroll(ui, &snapshot),
                AppScreen::Dashboard => self.draw_dashboard(ui, &snapshot),
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.request_shutdown();
    }
}

impl MeshApp {
    fn draw_first_run(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(28.0);
            ui.heading(RichText::new("Connect this PC to your mesh").size(28.0));
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "This application connects this PC directly to your other PCs. It will detect hardware and network automatically.",
                )
                .weak()
                .size(16.0),
            );
            ui.add_space(28.0);
        });

        ui.horizontal_centered(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(560.0, 320.0),
                Layout::top_down(Align::Center),
                |ui| {
                    card(ui, |ui| {
                        ui.heading("Create a new mesh");
                        ui.add_space(8.0);
                        ui.label("Start a mesh on this PC. You can invite other PCs next.");
                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            ui.label("PC name");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.display_name)
                                    .desired_width(300.0)
                                    .hint_text("This PC"),
                            );
                        });
                        ui.add_space(16.0);
                        if primary_button(ui, "Create a new mesh").clicked() {
                            self.send(UiCommand::CreateMesh {
                                display_name: self.display_name.clone(),
                            });
                        }
                    });

                    ui.add_space(16.0);

                    card(ui, |ui| {
                        ui.heading("Enroll this PC");
                        ui.add_space(8.0);
                        ui.label("Join an existing mesh with an invitation from another PC.");
                        ui.add_space(12.0);
                        if ui.button("Enroll this PC").clicked() {
                            self.send(UiCommand::OpenEnrollment);
                        }
                    });
                },
            );
        });
    }

    fn draw_enroll(&mut self, ui: &mut Ui, snapshot: &UiSnapshot) {
        ui.heading("Enroll this PC");
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "On a connected PC, choose “Add another PC.” Copy the invitation and paste it here.",
            )
            .weak(),
        );
        ui.add_space(16.0);

        card(ui, |ui| {
            ui.label("Invitation");
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.invitation_input)
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .hint_text("mesh1:..."),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "Join mesh").clicked() {
                    self.send(UiCommand::SubmitInvitation {
                        text: self.invitation_input.clone(),
                    });
                }
                if ui.button("Back").clicked() {
                    self.send(UiCommand::CancelEnrollment);
                }
            });
        });

        if !snapshot.enrollment.steps.is_empty() || snapshot.enrollment.error.is_some() {
            ui.add_space(16.0);
            card(ui, |ui| {
                ui.heading("Progress");
                ui.add_space(8.0);
                for step in &snapshot.enrollment.steps {
                    ui.label(format!("✓ {step}"));
                }
                if !snapshot.enrollment.current.is_empty()
                    && snapshot
                        .enrollment
                        .steps
                        .last()
                        .map(|step| step != &snapshot.enrollment.current)
                        .unwrap_or(true)
                {
                    ui.label(format!("• {}", snapshot.enrollment.current));
                }
                if let Some(error) = &snapshot.enrollment.error {
                    ui.add_space(8.0);
                    ui.colored_label(Color32::from_rgb(180, 60, 60), error);
                }
            });
        }
    }

    fn draw_dashboard(&mut self, ui: &mut Ui, snapshot: &UiSnapshot) {
        ui.heading("Dashboard");
        ui.add_space(4.0);
        ui.label(RichText::new("Local node is ready.").weak());
        ui.add_space(18.0);

        ui.columns(2, |columns| {
            card(&mut columns[0], |ui| {
                ui.heading("This PC");
                ui.add_space(10.0);
                kv(ui, "Name", &snapshot.local.display_name);
                kv(
                    ui,
                    "Node ID",
                    &snapshot
                        .local
                        .node_id
                        .map(|id| id.short_hex())
                        .unwrap_or_else(|| "—".to_owned()),
                );
                kv(
                    ui,
                    "Mesh ID",
                    &snapshot
                        .local
                        .mesh_id
                        .map(|id| id.short_hex())
                        .unwrap_or_else(|| "—".to_owned()),
                );
                kv(
                    ui,
                    "Listen",
                    &snapshot
                        .local
                        .listen_addr
                        .map(|addr| addr.to_string())
                        .unwrap_or_else(|| "—".to_owned()),
                );
                kv(ui, "Peers", &snapshot.peers.len().to_string());
            });

            card(&mut columns[1], |ui| {
                ui.heading("Connected PCs");
                ui.add_space(10.0);
                if snapshot.peers.is_empty() {
                    ui.label(RichText::new("No peers connected yet.").weak());
                } else {
                    for peer in &snapshot.peers {
                        ui.horizontal(|ui| {
                            let mark = if peer.connected { "●" } else { "○" };
                            ui.label(format!("{mark} {}", peer.display_name));
                        });
                    }
                }
                ui.add_space(12.0);
                if snapshot.can_create_invitation {
                    if primary_button(ui, "Add another PC").clicked() {
                        self.send(UiCommand::CreateInvitation);
                    }
                }
            });
        });

        if let Some(invitation) = &snapshot.enrollment.invitation_text {
            ui.add_space(16.0);
            card(ui, |ui| {
                ui.heading("Invitation");
                ui.add_space(8.0);
                ui.label("Copy this invitation and open it on the new PC.");
                ui.add_space(8.0);
                let mut invitation_text = invitation.clone();
                ui.add(
                    egui::TextEdit::multiline(&mut invitation_text)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy invitation").clicked() {
                        ui.ctx().copy_text(invitation.clone());
                    }
                    if ui.button("Clear").clicked() {
                        self.send(UiCommand::ClearInvitation);
                    }
                });
            });
        }
    }
}

fn style_visuals(ctx: &egui::Context) {
    for theme in [Theme::Dark, Theme::Light] {
        let mut style = (*ctx.style_of(theme)).clone();
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.visuals.window_corner_radius = 8.0.into();
        style.visuals.widgets.noninteractive.corner_radius = 6.0.into();
        style.visuals.widgets.inactive.corner_radius = 6.0.into();
        style.visuals.widgets.hovered.corner_radius = 6.0.into();
        style.visuals.widgets.active.corner_radius = 6.0.into();
        ctx.set_style_of(theme, style);
    }
}

fn card(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(16))
        .corner_radius(8.0)
        .show(ui, add_contents);
}

fn kv(ui: &mut Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).weak());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).strong());
        });
    });
}

fn primary_button(ui: &mut Ui, label: &str) -> egui::Response {
    let text = RichText::new(label).strong().color(Color32::WHITE);
    let button = egui::Button::new(text)
        .fill(Color32::from_rgb(36, 99, 235))
        .min_size(Vec2::new(180.0, 36.0))
        .corner_radius(6.0);
    ui.add(button)
}

fn phase_badge(ui: &mut Ui, phase: RuntimePhase) {
    let (label, color) = match phase {
        RuntimePhase::Starting => ("Starting", Color32::from_rgb(120, 120, 120)),
        RuntimePhase::AwaitingOnboarding => ("Setup", Color32::from_rgb(180, 120, 20)),
        RuntimePhase::Preparing => ("Preparing", Color32::from_rgb(180, 120, 20)),
        RuntimePhase::Connecting => ("Connecting", Color32::from_rgb(40, 110, 180)),
        RuntimePhase::Ready => ("Ready", Color32::from_rgb(30, 140, 70)),
        RuntimePhase::Failed => ("Failed", Color32::from_rgb(160, 50, 50)),
        RuntimePhase::ShuttingDown => ("Stopping", Color32::from_rgb(140, 60, 60)),
    };

    let response = ui.allocate_response(Vec2::new(96.0, 22.0), Sense::hover());
    let rect = response.rect;
    ui.painter()
        .rect_filled(rect, 11.0, color.gamma_multiply(0.18));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(13.0),
        color,
    );
}
