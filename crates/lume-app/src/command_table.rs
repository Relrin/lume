// The application's commands: every command a page may call, once, with the pages that
// may call it. This file is read twice. `build.rs` reads it to declare the commands to
// Tauri, which makes a permission for each, and to write the capability files that
// grant them (`capabilities/*.json`). `command_registry.rs` reads it to register the handlers
// and to grant the harness and the frame token theirs. A test writes the page's list
// of command names (`packages/ui/src/transport/commands.generated.ts`) from it.
//
// Adding a command: write the `#[tauri::command]` function, then add one line here with
// the pages that may call it, then build. Nothing else lists commands.
//
// The pages:
// - `terminal`: a window that shows terminals (`w-*`).
// - `settings`: the settings window.
// - `probe`: the window that reports on the webview or runs a test suite (`main`).
// - `harness`: granted at run time, only when the application runs a test suite.
// - `open`: may be called without the page's token (see `frame_token`).

commands! {
    frame_token::frame_token               => [open, terminal, settings, probe, harness];
    commands::webview_info                 => [terminal, settings, probe];
    commands::about_info                   => [settings];
    commands::probe_done                   => [probe];
    commands::carrier_open                 => [terminal, settings, harness];
    commands::carrier_send                 => [terminal, settings, harness];
    commands::session_spawn                => [harness];
    commands::session_attach               => [terminal, harness];
    commands::session_kill                 => [harness];
    commands::session_list                 => [harness];
    commands::clipboard_read               => [terminal];
    commands::clipboard_write              => [terminal, settings];
    commands::open_link                    => [terminal];
    commands::font_families                => [terminal, settings];
    commands::page_reply                   => [terminal, harness];
    commands::page_marks                   => [terminal];
    commands::page_shown                   => [terminal];
    commands_layout::layout_view           => [terminal];
    commands_layout::layout_apply          => [terminal];
    commands_layout::sessions_start        => [terminal];
    commands_layout::panes_busy            => [terminal];
    commands_layout::sessions_detached     => [terminal];
    commands_layout::commands_list         => [terminal, settings];
    commands_layout::command_run           => [terminal];
    commands_config::config_view           => [terminal, settings];
    commands_config::config_set            => [terminal, settings];
    commands_config::config_unset          => [terminal, settings];
    commands_config::config_open           => [terminal, settings];
    commands_config::config_schema         => [settings];
    commands_config::host_probe            => [terminal];
    commands_config::themes_list           => [settings];
    commands_config::shells_list           => [settings];
    commands_config::profiles_list         => [terminal, settings];
    commands_config::background_image      => [terminal];
    commands_window::window_drag           => [terminal, settings];
    commands_window::window_control        => [terminal, settings];
    commands_window::window_state          => [terminal, settings];
    commands_window::workspaces_list       => [terminal, settings];
    commands_folders::folders_list         => [terminal];
    rex::commands::rex_hosts               => [terminal, settings];
    rex::commands::rex_hosts_look          => [terminal];
    rex::commands::rex_read                => [terminal, settings];
    rex::commands::rex_reach               => [terminal, settings];
    rex::commands::rex_try                 => [terminal, settings];
    rex::commands::rex_host_put            => [terminal, settings];
    rex::commands::rex_host_remove         => [terminal, settings];
    commands_connections::ssh_reach        => [terminal, settings];
    commands_connections::ssh_try          => [terminal, settings];
    commands_connections::try_end          => [terminal, settings];
    commands_connections::ssh_profile_put  => [terminal, settings];
    commands_connections::profile_remove   => [terminal, settings];
    commands_connections::profiles_listed  => [terminal, settings];
    commands_plugins::ui_contributions     => [terminal];
    commands_plugins::ui_visible           => [terminal];
    commands_plugins::ui_assets            => [terminal];
    commands_plugins::ui_panel_expand      => [terminal];
    commands_plugins::view_opened          => [terminal, settings];
    commands_plugins::view_closed          => [terminal, settings];
    commands_plugins::view_call            => [terminal, settings];
    commands_plugins::view_state           => [terminal];
    commands_plugins::view_stream_open     => [terminal];
    commands_plugins::addons_list          => [settings];
    commands_plugins::addon_set_enabled    => [settings];
    commands_plugins::addon_remove         => [settings];
    commands_plugins::addon_revoke_machine => [settings];
    commands_plugins::addon_move           => [settings];
    commands_plugins::addon_inspect        => [settings];
    commands_plugins::addon_inspect_git    => [settings];
    commands_plugins::addon_check_update   => [settings];
    commands_plugins::addon_install        => [settings];
    commands_plugins::addon_cancel         => [settings];
    harness::harness_done                  => [harness];
    harness::harness_save                  => [harness];
    harness::harness_read                  => [harness];
}
