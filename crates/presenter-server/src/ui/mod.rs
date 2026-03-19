pub mod models;
pub mod scripts;
pub mod styles;
pub mod utils;

pub mod components;
pub mod home;
pub mod operator;
pub mod settings;
pub mod stage_design;
pub mod tablet;
pub mod timer_overlay;

// Note: stage_settings is deprecated - /ui/stage-settings redirects to /ui/stage-design

pub use home::render_home_ui;
pub use operator::render_operator_ui;
pub use settings::render_settings_ui;
pub use stage_design::render_stage_design_ui;
pub use tablet::render_tablet_ui;
pub use timer_overlay::render_timer_overlay;
