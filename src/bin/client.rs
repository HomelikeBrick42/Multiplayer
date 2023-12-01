use eframe::wgpu;
use multiplayer_game::App;

fn main() -> anyhow::Result<()> {
    eframe::run_native(
        "Multiplayer",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            vsync: false,
            hardware_acceleration: eframe::HardwareAcceleration::Preferred,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                present_mode: wgpu::PresentMode::AutoNoVsync,
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(|cc| Box::new(App::new(cc, false))),
    )?;
    Ok(())
}
