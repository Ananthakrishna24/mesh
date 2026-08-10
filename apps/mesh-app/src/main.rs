mod app;

use std::thread;

use mesh_node::NodeRuntime;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let runtime = NodeRuntime::create(default_display_name());
    let handle = runtime.handle();

    let worker = thread::Builder::new()
        .name("mesh-node".to_owned())
        .spawn(move || {
            let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            tokio_runtime.block_on(runtime.run());
        })
        .expect("mesh-node thread");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title("Mesh"),
        ..Default::default()
    };

    let result = eframe::run_native(
        "Mesh",
        options,
        Box::new(move |cc| Ok(Box::new(app::MeshApp::new(cc, handle)))),
    );

    let _ = worker.join();
    result
}

fn default_display_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "This PC".to_owned())
}
