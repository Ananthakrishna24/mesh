use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Theme, Ui, Vec2};
use mesh_core::{
    AppScreen, DeploymentId, ModelReference, NodeId, RecoveryAction, RuntimePhase, UiCommand,
    UiSnapshot,
};
use mesh_node::NodeHandle;

pub struct MeshApp {
    handle: NodeHandle,
    snapshots: tokio::sync::watch::Receiver<UiSnapshot>,
    display_name: String,
    invitation_input: String,
    hf_token_input: String,
    prompt_input: String,
    max_new_tokens: u32,
    temperature: f32,
    seed: u64,
    pipeline_deployment_id: String,
    pipeline_peer_node_id: Option<NodeId>,
    pipeline_local_stage_index: u16,
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
            hf_token_input: String::new(),
            prompt_input: "Say hello in one short sentence.".to_owned(),
            max_new_tokens: 64,
            temperature: 0.0,
            seed: 1,
            pipeline_deployment_id: DeploymentId::new().to_string(),
            pipeline_peer_node_id: None,
            pipeline_local_stage_index: 0,
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
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(12.0);
                    match snapshot.screen {
                        AppScreen::FirstRun => self.draw_first_run(ui),
                        AppScreen::Enroll => self.draw_enroll(ui, &snapshot),
                        AppScreen::Dashboard => self.draw_dashboard(ui, &snapshot),
                    }
                    ui.add_space(24.0);
                });
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

        if !snapshot.enrollment.steps.is_empty()
            || snapshot.enrollment.error.is_some()
            || snapshot.enrollment.recovery.is_some()
        {
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
                if let Some(recovery) = &snapshot.enrollment.recovery {
                    ui.add_space(12.0);
                    ui.label(RichText::new(&recovery.title).strong());
                    ui.label(&recovery.message);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let primary = match recovery.primary {
                            RecoveryAction::RetryAutomatic => "Try automatic setup again",
                            RecoveryAction::ShowManualSteps => "Show manual router steps",
                            RecoveryAction::RegenerateInvitation => "Create a new invitation",
                            RecoveryAction::OpenFirewallHelp => "Show firewall help",
                        };
                        if primary_button(ui, primary).clicked() {
                            match recovery.primary {
                                RecoveryAction::RetryAutomatic => {
                                    self.send(UiCommand::RetryAutomaticConnectivity);
                                }
                                RecoveryAction::ShowManualSteps => {
                                    self.send(UiCommand::ShowManualForwarding);
                                }
                                RecoveryAction::RegenerateInvitation => {
                                    self.send(UiCommand::CreateInvitation);
                                }
                                RecoveryAction::OpenFirewallHelp => {
                                    self.send(UiCommand::ShowFirewallHelp);
                                }
                            }
                        }
                        if let Some(secondary) = recovery.secondary {
                            let label = match secondary {
                                RecoveryAction::ShowManualSteps => "Show manual router steps",
                                RecoveryAction::OpenFirewallHelp => "Show firewall help",
                                RecoveryAction::RetryAutomatic => "Try again",
                                RecoveryAction::RegenerateInvitation => "New invitation",
                            };
                            if ui.button(label).clicked() {
                                match secondary {
                                    RecoveryAction::ShowManualSteps => {
                                        self.send(UiCommand::ShowManualForwarding);
                                    }
                                    RecoveryAction::OpenFirewallHelp => {
                                        self.send(UiCommand::ShowFirewallHelp);
                                    }
                                    RecoveryAction::RetryAutomatic => {
                                        self.send(UiCommand::RetryAutomaticConnectivity);
                                    }
                                    RecoveryAction::RegenerateInvitation => {
                                        self.send(UiCommand::CreateInvitation);
                                    }
                                }
                            }
                        }
                        if ui.button("Show firewall help").clicked() {
                            self.send(UiCommand::ShowFirewallHelp);
                        }
                    });
                    if recovery.show_firewall_help {
                        ui.add_space(8.0);
                        ui.label(RichText::new(&recovery.firewall_message).weak());
                    }
                    if recovery.show_manual {
                        if let Some(manual) = &recovery.manual {
                            ui.add_space(10.0);
                            ui.label(RichText::new("Manual UDP forwarding").strong());
                            kv(ui, "Protocol", &manual.protocol);
                            kv(ui, "Local UDP port", &manual.local_udp_port.to_string());
                            kv(
                                ui,
                                "Local address",
                                &manual
                                    .local_address
                                    .map(|addr| addr.to_string())
                                    .unwrap_or_else(|| "—".to_owned()),
                            );
                            for line in &manual.instructions {
                                ui.label(format!("• {line}"));
                            }
                            ui.add_space(8.0);
                            ui.label("Public address after forwarding");
                            let mut public = manual.public_address_input.clone();
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut public)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("203.0.113.10:4433"),
                                )
                                .changed()
                            {
                                self.send(UiCommand::SetManualPublicAddress { address: public });
                            }
                            if ui.button("Save manual address").clicked() {
                                self.send(UiCommand::ApplyManualPublicAddress);
                            }
                        }
                    }
                    if !recovery.technical_details.is_empty() {
                        ui.add_space(8.0);
                        ui.collapsing("Technical details", |ui| {
                            for detail in &recovery.technical_details {
                                ui.monospace(detail);
                            }
                        });
                    }
                }
                if let Some(mapping_ok) = snapshot.enrollment.router_mapping_ok {
                    ui.add_space(6.0);
                    if mapping_ok {
                        ui.label(RichText::new("Router mapping prepared automatically.").weak());
                    } else {
                        ui.label(
                            RichText::new(
                                "Automatic router mapping was unavailable on this network.",
                            )
                            .weak(),
                        );
                    }
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
                ui.heading("Hardware");
                ui.add_space(10.0);
                if let Some(hardware) = &snapshot.hardware {
                    kv(ui, "CPU", &hardware.cpu_model);
                    kv(ui, "Cores", &hardware.cpu_logical_cores.to_string());
                    kv(
                        ui,
                        "Memory",
                        &format!(
                            "{} free / {}",
                            mesh_core::format_bytes(hardware.memory_available_bytes),
                            mesh_core::format_bytes(hardware.memory_total_bytes)
                        ),
                    );
                    kv(
                        ui,
                        "Disk",
                        &format!(
                            "{} free / {}",
                            mesh_core::format_bytes(hardware.disk_available_bytes),
                            mesh_core::format_bytes(hardware.disk_total_bytes)
                        ),
                    );
                    kv(
                        ui,
                        "CPU probe",
                        &format!("{:.2} GFLOP/s", hardware.cpu_fp32_gflops),
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new("GPUs").strong());
                    for line in &hardware.gpu_lines {
                        ui.label(line);
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new(&hardware.status).weak());
                } else {
                    ui.label(RichText::new("Hardware report unavailable.").weak());
                }
                ui.add_space(10.0);
                if ui.button("Refresh hardware").clicked() {
                    self.send(UiCommand::RefreshHardware);
                }
            });
        });

        ui.add_space(16.0);
        card(ui, |ui| {
            ui.heading("Local resources");
            ui.add_space(10.0);
            kv(ui, "Capacity", &snapshot.resources.capacity_line);
            kv(ui, "Available", &snapshot.resources.available_line);
            ui.add_space(6.0);
            if snapshot.resources.active.is_empty() {
                ui.label(RichText::new("No active reservations.").weak());
            } else {
                ui.label(RichText::new("Active reservations").strong());
                for item in &snapshot.resources.active {
                    ui.label(format!(
                        "{} · {} · {}",
                        item.state.as_str(),
                        item.purpose,
                        item.amount_line
                    ));
                    ui.label(
                        RichText::new(format!(
                            "owner {} · deploy {} · expires {}",
                            item.owner_node_id.short_hex(),
                            item.deployment_id.short_hex(),
                            item.expires_at_unix_ms
                        ))
                        .weak(),
                    );
                    ui.add_space(6.0);
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Probe local reservation").clicked() {
                    self.send(UiCommand::RunLocalReservationProbe);
                }
                if ui.button("Release all").clicked() {
                    self.send(UiCommand::ReleaseAllLocalReservations);
                }
            });
        });

        ui.add_space(16.0);
        card(ui, |ui| {
            ui.heading("Models");
            ui.add_space(10.0);
            let models = &snapshot.models;
            kv(
                ui,
                "Provider",
                &format!(
                    "huggingface · {} · {}",
                    models.provider_access.status.as_str(),
                    models.provider_access.auth_mode.as_str()
                ),
            );
            kv(ui, "Access", &models.provider_access.detail);
            kv(
                ui,
                "Selected",
                models.selected_model.as_deref().unwrap_or("none"),
            );
            if let Some(identity) = &models.resolved_identity {
                kv(ui, "Revision", &identity.revision);
                kv(
                    ui,
                    "Manifest",
                    &identity.manifest_hash[..identity.manifest_hash.len().min(16)],
                );
            }
            kv(ui, "Status", &models.status_line);
            if let Some(progress) = &models.progress {
                let total = progress
                    .bytes_total
                    .map(mesh_core::format_bytes)
                    .unwrap_or_else(|| "?".to_owned());
                kv(
                    ui,
                    "Download",
                    &format!(
                        "{} · {} / {} · {}",
                        progress.phase,
                        mesh_core::format_bytes(progress.bytes_done),
                        total,
                        progress.artifact_path
                    ),
                );
            }
            if let Some(summary) = &models.last_prepare_summary {
                ui.label(RichText::new(summary).weak());
            }
            if let Some(error) = &models.error {
                ui.colored_label(Color32::from_rgb(180, 60, 60), error);
            }
            kv(
                ui,
                "Cache",
                &format!(
                    "{} used · {} entries · {}",
                    mesh_core::format_bytes(models.cache.used_bytes),
                    models.cache.entry_count,
                    if models.cache.root.is_empty() {
                        "—"
                    } else {
                        &models.cache.root
                    }
                ),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!models.busy, egui::Button::new("Select Qwen3-4B"))
                    .clicked()
                {
                    self.send(UiCommand::SelectModel {
                        reference: ModelReference::qwen3_4b(),
                    });
                }
                if ui
                    .add_enabled(!models.busy, egui::Button::new("Select Qwen3-8B"))
                    .clicked()
                {
                    self.send(UiCommand::SelectModel {
                        reference: ModelReference::qwen3_8b(),
                    });
                }
                if ui
                    .add_enabled(!models.busy, egui::Button::new("Check access"))
                    .clicked()
                {
                    self.send(UiCommand::RefreshProviderAccess);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !models.busy && models.selected_reference.is_some(),
                        egui::Button::new("Probe / resolve"),
                    )
                    .clicked()
                {
                    self.send(UiCommand::ProbeSelectedModel);
                }
                if ui
                    .add_enabled(
                        !models.busy && models.resolved_identity.is_some(),
                        egui::Button::new("Prepare downloads"),
                    )
                    .clicked()
                {
                    self.send(UiCommand::PrepareSelectedModel);
                }
                if ui
                    .add_enabled(models.busy, egui::Button::new("Cancel"))
                    .clicked()
                {
                    self.send(UiCommand::CancelModelWork);
                }
                if ui
                    .add_enabled(!models.busy, egui::Button::new("Clear cache"))
                    .clicked()
                {
                    self.send(UiCommand::ClearModelCache);
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Hugging Face token (optional for public models)").weak());
            ui.add(
                egui::TextEdit::singleline(&mut self.hf_token_input)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("hf_..."),
            );
            ui.horizontal(|ui| {
                if ui.button("Save token").clicked() {
                    self.send(UiCommand::SaveHuggingFaceToken {
                        token: self.hf_token_input.clone(),
                    });
                    self.hf_token_input.clear();
                }
                if ui.button("Delete token").clicked() {
                    self.send(UiCommand::DeleteHuggingFaceToken);
                    self.hf_token_input.clear();
                }
            });
        });

        ui.add_space(16.0);
        card(ui, |ui| {
            ui.heading("Inference");
            ui.add_space(10.0);
            let inference = &snapshot.inference;
            kv(
                ui,
                "Phase",
                inference
                    .phase
                    .map(|phase| phase.as_str())
                    .unwrap_or("idle"),
            );
            kv(
                ui,
                "Model",
                inference.model_line.as_deref().unwrap_or("none"),
            );
            kv(ui, "Backend", inference.backend.as_deref().unwrap_or("—"));
            kv(
                ui,
                "Routed to",
                inference.routed_node_id.as_deref().unwrap_or("—"),
            );
            kv(ui, "Status", &inference.status_line);
            kv(
                ui,
                "Deployment",
                inference.deployment_id.as_deref().unwrap_or("—"),
            );
            if let Some(error) = &inference.error {
                ui.colored_label(Color32::from_rgb(180, 60, 60), error);
            }
            if !inference.replicas.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Replicas").strong());
                for replica in &inference.replicas {
                    ui.label(RichText::new(replica.status_line()).weak());
                }
            }
            if !inference.output_text.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Output").strong());
                ui.label(&inference.output_text);
            }
            if let Some(reason) = &inference.stop_reason {
                kv(ui, "Stop", reason);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let can_load = !inference.busy
                    && snapshot.models.last_prepare_summary.is_some()
                    && snapshot.models.resolved_identity.is_some();
                if ui
                    .add_enabled(can_load, egui::Button::new("Load model"))
                    .clicked()
                {
                    self.send(UiCommand::LoadSelectedModel);
                }
                if ui
                    .add_enabled(
                        !inference.busy && inference.model_line.is_some(),
                        egui::Button::new("Unload"),
                    )
                    .clicked()
                {
                    self.send(UiCommand::UnloadModel);
                }
                if ui
                    .add_enabled(inference.busy, egui::Button::new("Cancel generation"))
                    .clicked()
                {
                    self.send(UiCommand::CancelGeneration);
                }
            });
            ui.add_space(8.0);
            ui.collapsing("Two-PC pipeline placement", |ui| {
                ui.label(
                    RichText::new(
                        "Use the same deployment ID on both PCs. Choose First on one PC and Final on the other.",
                    )
                    .weak(),
                );
                ui.horizontal(|ui| {
                    ui.label("Deployment ID");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.pipeline_deployment_id)
                            .desired_width(280.0),
                    );
                    if ui.button("New ID").clicked() {
                        self.pipeline_deployment_id = DeploymentId::new().to_string();
                    }
                });

                let selected_peer = self.pipeline_peer_node_id.and_then(|node_id| {
                    snapshot
                        .peers
                        .iter()
                        .find(|peer| peer.connected && peer.node_id == node_id)
                });
                if selected_peer.is_none() {
                    self.pipeline_peer_node_id = snapshot
                        .peers
                        .iter()
                        .find(|peer| peer.connected)
                        .map(|peer| peer.node_id);
                }
                let peer_label = self
                    .pipeline_peer_node_id
                    .and_then(|node_id| {
                        snapshot
                            .peers
                            .iter()
                            .find(|peer| peer.connected && peer.node_id == node_id)
                    })
                    .map(|peer| format!("{} ({})", peer.display_name, peer.node_id.short_hex()))
                    .unwrap_or_else(|| "No connected peer".to_owned());
                egui::ComboBox::from_id_salt("pipeline_peer")
                    .selected_text(peer_label)
                    .show_ui(ui, |ui| {
                        for peer in snapshot.peers.iter().filter(|peer| peer.connected) {
                            ui.selectable_value(
                                &mut self.pipeline_peer_node_id,
                                Some(peer.node_id),
                                format!("{} ({})", peer.display_name, peer.node_id.short_hex()),
                            );
                        }
                    });

                ui.horizontal(|ui| {
                    ui.label("This PC runs");
                    ui.selectable_value(
                        &mut self.pipeline_local_stage_index,
                        0,
                        "First stage",
                    );
                    ui.selectable_value(
                        &mut self.pipeline_local_stage_index,
                        1,
                        "Final stage",
                    );
                });

                let nodes = ordered_pipeline_nodes(
                    snapshot.local.node_id,
                    self.pipeline_peer_node_id,
                    self.pipeline_local_stage_index,
                );
                let deployment_valid =
                    DeploymentId::parse_hex(self.pipeline_deployment_id.trim()).is_ok();
                let can_load_stage = !inference.busy
                    && !snapshot.models.busy
                    && snapshot.models.resolved_identity.is_some()
                    && deployment_valid
                    && nodes.is_some();
                if ui
                    .add_enabled(can_load_stage, egui::Button::new("Load this PC's stage"))
                    .clicked()
                {
                    let node_ids = nodes
                        .expect("enabled pipeline load has a node order")
                        .into_iter()
                        .map(|node_id| node_id.to_string())
                        .collect();
                    self.send(UiCommand::LoadPipelineStage {
                        deployment_id: self.pipeline_deployment_id.trim().to_owned(),
                        stage_index: self.pipeline_local_stage_index,
                        node_ids,
                    });
                }
                if !deployment_valid {
                    ui.label(RichText::new("Deployment ID must be 32 hexadecimal characters.").weak());
                } else if snapshot.local.node_id.is_none() {
                    ui.label(RichText::new("Local node identity is not ready.").weak());
                } else if self.pipeline_peer_node_id.is_none() {
                    ui.label(RichText::new("Connect a peer before loading a two-stage pipeline.").weak());
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Prompt").weak());
            ui.add(
                egui::TextEdit::multiline(&mut self.prompt_input)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            ui.horizontal(|ui| {
                ui.label("max tokens");
                ui.add(egui::DragValue::new(&mut self.max_new_tokens).range(1..=512));
                ui.label("temperature");
                ui.add(
                    egui::DragValue::new(&mut self.temperature)
                        .range(0.0..=2.0)
                        .speed(0.05),
                );
                ui.label("seed");
                ui.add(egui::DragValue::new(&mut self.seed));
            });
            let can_generate = !inference.busy
                && (inference.model_line.is_some()
                    || inference
                        .replicas
                        .iter()
                        .any(|replica| replica.can_accept()));
            if ui
                .add_enabled(can_generate, egui::Button::new("Generate"))
                .clicked()
            {
                self.send(UiCommand::Generate {
                    prompt: self.prompt_input.clone(),
                    max_new_tokens: self.max_new_tokens,
                    temperature: self.temperature,
                    seed: self.seed,
                });
            }
        });

        ui.add_space(16.0);
        card(ui, |ui| {
            ui.heading("Connected PCs");
            ui.add_space(10.0);
            if snapshot.peers.is_empty() {
                ui.label(RichText::new("No peers connected yet.").weak());
            } else {
                for peer in &snapshot.peers {
                    let mark = if peer.connected { "●" } else { "○" };
                    ui.label(RichText::new(format!("{mark} {}", peer.display_name)).strong());
                    if let Some(line) = &peer.hardware_line {
                        ui.label(RichText::new(line).weak());
                    }
                    let link_view = mesh_core::LinkSummaryView::from_measurement(
                        peer.link.as_ref(),
                        mesh_core::now_unix_ms(),
                    );
                    kv(ui, "Delay", &link_view.delay_label());
                    kv(ui, "To peer", &link_view.bandwidth_label("to"));
                    kv(ui, "From peer", &link_view.bandwidth_label("from"));
                    kv(
                        ui,
                        "Stability",
                        &link_view
                            .stability_score
                            .map(|score| score.to_string())
                            .unwrap_or_else(|| "unavailable".to_owned()),
                    );
                    if let Some(model) = &peer.replica_model_line {
                        kv(
                            ui,
                            "Replica",
                            &format!(
                                "{} · {} · {}/{} · {}",
                                model,
                                peer.replica_backend.as_deref().unwrap_or("?"),
                                peer.replica_active_requests,
                                peer.replica_max_concurrent_requests.max(1),
                                if peer.replica_ready { "ready" } else { "busy" }
                            ),
                        );
                    }
                    ui.add_space(10.0);
                }
            }
            if snapshot.can_create_invitation {
                if primary_button(ui, "Add another PC").clicked() {
                    self.send(UiCommand::CreateInvitation);
                }
            }
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

fn ordered_pipeline_nodes(
    local: Option<NodeId>,
    peer: Option<NodeId>,
    local_stage_index: u16,
) -> Option<[NodeId; 2]> {
    let local = local?;
    let peer = peer?;
    match local_stage_index {
        0 => Some([local, peer]),
        1 => Some([peer, local]),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_node_order_matches_local_stage() {
        let local = NodeId::from_bytes([1; 32]);
        let peer = NodeId::from_bytes([2; 32]);

        assert_eq!(
            ordered_pipeline_nodes(Some(local), Some(peer), 0),
            Some([local, peer])
        );
        assert_eq!(
            ordered_pipeline_nodes(Some(local), Some(peer), 1),
            Some([peer, local])
        );
        assert_eq!(ordered_pipeline_nodes(Some(local), Some(peer), 2), None);
    }
}
