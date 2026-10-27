//! The Settings page.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::i18n::{gettext, pgettext};
use crate::model::{Action, Dialog};
use crate::settings::{LanguageChoice, ProxyMode, ThemeChoice};
use crate::theme::{self, Icon, Palette};

use super::widgets;

const PLAYBACK_DIRTY_ID: &str = "playback-settings-dirty";
const SETTINGS_FILTER_ID: &str = "settings-filter";

/// Whether a settings row matches the filter query (case-insensitive).
/// An empty query matches everything, so the page reads exactly as
/// before when the search field is empty.
pub(crate) fn row_matches(needle: &str, title: &str, description: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&needle) || description.to_lowercase().contains(&needle)
}

/// Text shared by section selection and the rendered row. Conditional rows
/// participate only on the platforms and account states where they exist.
struct RowText<'a> {
    title: std::borrow::Cow<'a, str>,
    description: std::borrow::Cow<'a, str>,
    available: bool,
}

impl<'a> RowText<'a> {
    fn new(
        title: impl Into<std::borrow::Cow<'a, str>>,
        description: impl Into<std::borrow::Cow<'a, str>>,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            available: true,
        }
    }

    fn when(mut self, available: bool) -> Self {
        self.available = available;
        self
    }

    fn matches(&self, needle: &str, section: &str) -> bool {
        self.available
            && (needle.is_empty()
                || section.to_lowercase().contains(needle)
                || row_matches(needle, &self.title, &self.description))
    }
}

/// The guide to writing a palette file for the themes folder.
const THEMES_GUIDE_URL: &str =
    "https://github.com/nekomario28/apple-music-linux/blob/master/crates/melba/docs/themes.md";

fn section_matches(needle: &str, title: &str, rows: &[RowText<'_>]) -> bool {
    let needle = needle.trim().to_lowercase();
    rows.iter().any(|row| row.matches(&needle, title))
}

fn filtered_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    needle: &str,
    section: &str,
    row: &RowText<'_>,
    control: impl FnOnce(&mut egui::Ui),
) {
    if row.matches(needle, section) {
        widgets::setting_row(ui, palette, &row.title, &row.description, control);
    }
}

/// `filtered_row` for a control that needs `control_width` points.
fn filtered_row_sized(
    ui: &mut egui::Ui,
    palette: &Palette,
    needle: &str,
    section: &str,
    row: &RowText<'_>,
    control_width: f32,
    control: impl FnOnce(&mut egui::Ui),
) {
    if row.matches(needle, section) {
        widgets::setting_row_sized(
            ui,
            palette,
            &row.title,
            &row.description,
            control_width,
            control,
        );
    }
}

/// Forget the search text — demo flows use it to reset the filter.
#[cfg(test)]
pub(crate) fn clear_search(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<String>(egui::Id::new(SETTINGS_FILTER_ID)));
}
const PROXY_DIRTY_ID: &str = "proxy-settings-dirty";

fn section(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(10.0);
    theme::text(ui, title, theme::bold(18.0), palette.text);
    ui.add_space(8.0);
    Frame::new()
        .fill(
            palette
                .surface
                .gamma_multiply(if palette.dark { 0.7 } else { 1.0 }),
        )
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 2))
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(760.0));
            add_contents(ui);
        });
    ui.add_space(8.0);
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    ui.add_space(8.0);
    theme::text(
        ui,
        gettext(locale, "Settings"),
        theme::bold(28.0),
        palette.text,
    );
    ui.add_space(4.0);
    let mut filter = ui
        .data(|data| data.get_temp::<String>(egui::Id::new(SETTINGS_FILTER_ID)))
        .unwrap_or_default();
    widgets::search_field(
        ui,
        &palette,
        app.locale,
        egui::Id::new("settings-search-field"),
        &mut filter,
        &gettext(locale, "Search settings"),
        ui.available_width().min(400.0),
    );
    let needle = filter.trim().to_lowercase();
    ui.data_mut(|data| data.insert_temp(egui::Id::new(SETTINGS_FILTER_ID), filter));
    ui.add_space(4.0);
    let dirty_id = egui::Id::new(PLAYBACK_DIRTY_ID);
    let playback_dirty = ui
        .data(|data| data.get_temp::<bool>(dirty_id))
        .unwrap_or(false);
    let proxy_dirty_id = egui::Id::new(PROXY_DIRTY_ID);
    let mut proxy_dirty = ui
        .data(|data| data.get_temp::<bool>(proxy_dirty_id))
        .unwrap_or(false);
    let mut changed = false;
    let mut any_visible = false;
    let account = gettext(locale, "Account");
    let sign_out = gettext(locale, "Sign out");
    let account_rows = [RowText::new(sign_out.clone(), account.clone())];
    if section_matches(&needle, &account, &account_rows) {
        any_visible = true;
        section(ui, &palette, &account, |ui| {
            if row_matches(&needle, &format!("{sign_out} {account}"), "") {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let avatar = app.user.as_ref().and_then(|user| {
                        pick_image(&user.images, app.art_px(56.0)).map(str::to_string)
                    });
                    widgets::cover(ui, &palette, avatar.as_deref(), 56.0, 28.0, Icon::User);
                    ui.vertical(|ui| {
                        let name = app
                            .user
                            .as_ref()
                            .map(|user| user.name().to_string())
                            .unwrap_or_default();
                        theme::text(ui, name, theme::semibold(16.0), palette.text);
                        let product = app
                            .user
                            .as_ref()
                            .and_then(|user| user.product.clone())
                            .map(|product| match product.as_str() {
                                "premium" => "Apple Music".to_string(),
                                "free" | "open" => {
                                    gettext(locale, "Playback needs an Apple Music subscription")
                                        .into_owned()
                                }
                                other => other.to_string(),
                            })
                            .unwrap_or_default();
                        theme::text(ui, product, theme::regular(13.0), palette.secondary);
                        if let Some(username) =
                            app.local.connected.then(|| app.local.username.clone())
                            && !username.is_empty()
                        {
                            theme::text(
                                ui,
                                // Translators: {username} is the Apple Music account's user name.
                                gettext(locale, "Connected as {username}")
                                    .replace("{username}", &username),
                                theme::regular(12.0),
                                palette.dim,
                            );
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::pill_button(ui, &palette, &sign_out, false).clicked() {
                            app.actions.push(Action::SignOut);
                        }
                    });
                });
                ui.add_space(10.0);
            }
        });
    }

    let audio_output = gettext(locale, "Audio output");
    let audio_output_rows = [RowText::new(
        audio_output.clone(),
        gettext(
            locale,
            "Direct hardware output (hw:X,Y) can be bit-perfect when its rate, depth and channels match. The OS mixer is never bit-perfect.",
        ),
    )];
    if section_matches(&needle, &audio_output, &audio_output_rows) {
        any_visible = true;
        section(ui, &palette, &audio_output, |ui| {
            filtered_row(
                ui,
                &palette,
                &needle,
                &audio_output,
                &audio_output_rows[0],
                |ui| {
                    if app.audio_outputs.is_none()
                        && !app.audio_outputs_loading
                        && app.audio_outputs_error.is_none()
                    {
                        app.actions.push(Action::LoadAudioOutputs);
                    }
                    let outputs = app.audio_outputs.clone();
                    let current = outputs
                        .as_ref()
                        .map(|outputs| outputs.current.clone())
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "auto".into());
                    let selected = if current == "auto" {
                        gettext(locale, "Auto (prefer USB/analog)").into_owned()
                    } else if current == "default" {
                        gettext(locale, "OS mixer (not bit-perfect)").into_owned()
                    } else {
                        current.clone()
                    };
                    let mut choose = None;
                    egui::ComboBox::from_id_salt("audio-output-device")
                        .selected_text(selected)
                        .width(260.0_f32.min(ui.available_width()))
                        .show_ui(ui, |ui| {
                            for (value, label) in [
                                (
                                    "auto",
                                    gettext(locale, "Auto (prefer USB/analog)").into_owned(),
                                ),
                                (
                                    "default",
                                    gettext(locale, "OS mixer (not bit-perfect)").into_owned(),
                                ),
                            ] {
                                if ui.selectable_label(current == value, label).clicked() {
                                    choose = Some(value.to_string());
                                }
                            }
                            if let Some(outputs) = &outputs {
                                for device in outputs.devices.iter().filter(|device| device.usable())
                                {
                                    let busy = !device.openable;
                                    let label = if busy {
                                        format!(
                                            "{} ({}) — {}",
                                            device.card_name,
                                            device.device,
                                            gettext(locale, "in use")
                                        )
                                    } else {
                                        format!("{} ({})", device.card_name, device.device)
                                    };
                                    let response = ui
                                        .selectable_label(current == device.device, label)
                                        .on_hover_text(if busy {
                                            gettext(locale, "Selecting this output will stop the current PipeWire audio and give Melba exclusive control.")
                                        } else {
                                            gettext(locale, "Use this direct hardware output.")
                                        });
                                    if response.clicked() {
                                        choose = Some(device.device.clone());
                                    }
                                }
                            }
                        });
                    if let Some(device) = choose {
                        app.actions.push(Action::SetAudioDevice(device));
                    }
                    if theme::soft_button(
                        ui,
                        &palette,
                        Some(Icon::Refresh),
                        &gettext(locale, "Refresh outputs"),
                        false,
                    )
                    .clicked()
                    {
                        app.actions.push(Action::LoadAudioOutputs);
                    }
                    if app.audio_outputs_loading {
                        widgets::loading_row(ui, &palette, app.locale);
                    }
                    if let Some(error) = &app.audio_outputs_error {
                        theme::subtle(ui, &palette, error);
                    }
                },
            );
        });
    }

    let audio_cache = gettext(locale, "Audio cache");
    let appearance = gettext(locale, "Appearance");
    let theme_title = gettext(locale, "Theme");
    let theme_guide = gettext(locale, "How to make a theme");
    let themes_folder = gettext(locale, "Open themes folder");
    let accent_from_art = gettext(locale, "Colour from album art");
    let sidebar_compact = gettext(locale, "Compact library sidebar");
    let tracklist_compact = gettext(locale, "Compact track list");
    let middle_click = gettext(locale, "Middle-click autoscroll");
    let custom_titlebar = gettext(locale, "Custom title bar");
    let show_custom_titlebar = cfg!(target_os = "linux") || app.windows_controls_visible();
    let appearance_rows = [
        RowText::new(theme_title.clone(), {
            let detail = theme::catalog_detail(
                &app.custom_themes,
                locale,
                app.settings.custom_theme.as_deref(),
            );
            if !detail.is_empty() {
                detail
            } else if app.custom_themes.follows_omarchy() {
                gettext(locale, "Follow system uses your Omarchy colours.")
            } else {
                gettext(
                    locale,
                    "Follow system uses your desktop's light or dark appearance.",
                )
            }
        }),
        RowText::new(
            gettext(locale, "Language"),
            gettext(
                locale,
                "System follows your computer's language. Untranslated text stays in English.",
            ),
        ),
        RowText::new(
            accent_from_art.clone(),
            gettext(
                locale,
                "Use the current cover's colour on pages and the player bar.",
            ),
        ),
        RowText::new(
            sidebar_compact.clone(),
            gettext(locale, "Show names without covers in the sidebar."),
        ),
        RowText::new(
            tracklist_compact.clone(),
            gettext(locale, "Show each track on one line without a cover."),
        ),
        RowText::new(
            gettext(locale, "Interface zoom"),
            super::keys::platform_shortcut(
                &gettext(
                    locale,
                    "Ctrl+Plus and Ctrl+Minus work anywhere; Ctrl+0 resets.",
                ),
                &gettext(
                    locale,
                    "Cmd+Plus and Cmd+Minus work anywhere; Cmd+0 resets.",
                ),
            )
            .to_owned(),
        ),
        RowText::new(
            middle_click.clone(),
            gettext(
                locale,
                "Middle-click a list, then move the pointer to scroll it. Off by default, because a middle click usually pastes on Linux.",
            ),
        )
        .when(cfg!(target_os = "linux")),
        RowText::new(
            custom_titlebar.clone(),
            gettext(
                locale,
                "Draw Melba's own title bar and window buttons instead of the desktop's.",
            ),
        )
        .when(show_custom_titlebar),
    ];
    if section_matches(&needle, &appearance, &appearance_rows) {
        any_visible = true;
        section(ui, &palette, &appearance, |ui| {
            // Wide enough for the theme's two buttons side by side.
            let theme_buttons_width = theme::soft_button_width(ui, &theme_guide)
                + theme::soft_button_width(ui, &themes_folder)
                + 6.0;
            filtered_row_sized(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[0],
                theme_buttons_width,
                |ui| {
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        let selected = app
                            .settings
                            .custom_theme
                            .as_deref()
                            .map(|filename| fastframe_theme::display_name(filename).into())
                            .unwrap_or_else(|| app.settings.theme.label(locale));
                        let response = egui::ComboBox::from_id_salt("appearance_theme")
                            .selected_text(selected.as_ref())
                            .width(200.0_f32.min(ui.available_width()))
                            .show_ui(ui, |ui| {
                                for choice in ThemeChoice::ALL {
                                    if ui
                                        .selectable_label(
                                            app.settings.custom_theme.is_none()
                                                && app.settings.theme == choice,
                                            choice.label(locale).as_ref(),
                                        )
                                        .clicked()
                                    {
                                        app.actions.push(Action::SetTheme(choice));
                                    }
                                }
                                if app.custom_themes.picker_themes().next().is_some() {
                                    ui.separator();
                                }
                                for theme in app.custom_themes.picker_themes() {
                                    if ui
                                        .selectable_label(
                                            app.settings.custom_theme.as_deref()
                                                == Some(theme.filename.as_str()),
                                            fastframe_theme::display_name(&theme.filename),
                                        )
                                        .clicked()
                                    {
                                        app.actions
                                            .push(Action::SetCustomTheme(theme.filename.clone()));
                                    }
                                }
                            });
                        response.response.widget_info(|| {
                            let mut info = egui::WidgetInfo::labeled(
                                egui::WidgetType::ComboBox,
                                ui.is_enabled(),
                                theme_title.as_ref(),
                            );
                            info.current_text_value = Some(selected.to_string());
                            info
                        });
                        // The guide to writing a theme sits beside the folder
                        // it goes in, and above it when both do not fit.
                        let (guide, folder) = (&theme_guide, &themes_folder);
                        let gap = 6.0;
                        let mut buttons = |ui: &mut egui::Ui| {
                            if theme::soft_button(ui, &palette, Some(Icon::Globe), guide, false)
                                .clicked()
                            {
                                app.actions.push(Action::OpenUrl(THEMES_GUIDE_URL.into()));
                            }
                            if theme::soft_button(
                                ui,
                                &palette,
                                Some(Icon::ExternalLink),
                                folder,
                                false,
                            )
                            .clicked()
                            {
                                app.actions.push(Action::OpenThemesFolder);
                            }
                        };
                        let both = theme::soft_button_width(ui, guide)
                            + theme::soft_button_width(ui, folder)
                            + gap;
                        if ui.available_width() >= both {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                buttons(ui);
                            });
                        } else {
                            ui.spacing_mut().item_spacing.y = gap;
                            buttons(ui);
                        }
                    });
                },
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[1],
                |ui| language_picker(app, ui),
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[2],
                |ui| {
                    if widgets::switch(
                        ui,
                        &palette,
                        &accent_from_art,
                        &mut app.settings.accent_from_art,
                    )
                    .changed()
                    {
                        changed = true;
                    }
                },
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[3],
                |ui| {
                    if widgets::switch(
                        ui,
                        &palette,
                        &sidebar_compact,
                        &mut app.settings.sidebar_compact,
                    )
                    .changed()
                    {
                        changed = true;
                    }
                },
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[4],
                |ui| {
                    if widgets::switch(
                        ui,
                        &palette,
                        &tracklist_compact,
                        &mut app.settings.tracklist_compact,
                    )
                    .changed()
                    {
                        changed = true;
                    }
                },
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[5],
                |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let mut zoom = app.settings.zoom;
                        if theme::soft_button(ui, &palette, None, "+", false).clicked() {
                            zoom = (zoom + 0.1).min(2.5);
                        }
                        theme::text(
                            ui,
                            format!("{:.0}%", zoom * 100.0),
                            theme::medium(13.5),
                            palette.text,
                        );
                        if theme::soft_button(ui, &palette, None, "-", false).clicked() {
                            zoom = (zoom - 0.1).max(0.5);
                        }
                        if (zoom - app.settings.zoom).abs() > 0.001 {
                            app.settings.zoom = zoom;
                            ui.ctx().set_zoom_factor(zoom);
                            app.mark_settings_dirty();
                        }
                    });
                },
            );
            if cfg!(target_os = "linux") {
                filtered_row(
                    ui,
                    &palette,
                    &needle,
                    &appearance,
                    &appearance_rows[6],
                    |ui| {
                        if widgets::switch(
                            ui,
                            &palette,
                            &middle_click,
                            &mut app.settings.middle_click_autoscroll,
                        )
                        .changed()
                        {
                            changed = true;
                        }
                    },
                );
            }
            if show_custom_titlebar {
                filtered_row(
                    ui,
                    &palette,
                    &needle,
                    &appearance,
                    &appearance_rows[7],
                    |ui| {
                        let mut custom = app.settings.custom_titlebar;
                        if widgets::switch(ui, &palette, &custom_titlebar, &mut custom).changed() {
                            app.actions.push(Action::SetCustomTitlebar(custom));
                        }
                    },
                );
            }
        });
    }

    let proxy = gettext(locale, "Proxy");
    let proxy_rows = [RowText::new(
        gettext(locale, "Mode, host, port, username and password"),
        gettext(
            locale,
            "Off, System, HTTP or SOCKS5 proxy for network requests.",
        ),
    )];
    if section_matches(&needle, &proxy, &proxy_rows) {
        any_visible = true;
        ui.push_id("proxy-settings", |ui| {
    section(ui, &palette, &proxy, |ui| {
        widgets::setting_row(
            ui,
            &palette,
            &pgettext(locale, "proxy", "Mode"),
            &gettext(
                locale,
                "Off ignores environment variables. System uses them, and the OS proxy on macOS and Windows.",
            ),
            |_| {},
        );
        // The row's control slot is right-to-left and too narrow for four
        // choices; they sit on this line so they read Off, System, HTTP, SOCKS5.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for choice in ProxyMode::ALL {
                if theme::soft_button(
                    ui,
                    &palette,
                    None,
                    &choice.label(locale),
                    app.settings.proxy_mode == choice,
                )
                .clicked()
                    && app.settings.proxy_mode != choice
                {
                    app.settings.proxy_mode = choice;
                    changed = true;
                    app.actions.push(Action::ProxyEdited);
                    if choice.is_manual() {
                        proxy_dirty = true;
                    } else {
                        proxy_dirty = false;
                        app.actions.push(Action::ApplyProxy);
                    }
                }
            }
        });
        ui.add_space(10.0);
        if app.settings.proxy_mode.is_manual() {
            if widgets::proxy_manual_form(
                ui,
                &palette,
                app.locale,
                &mut app.settings.proxy_host,
                &mut app.settings.proxy_port,
                &mut app.settings.proxy_username,
                &mut app.settings.proxy_password,
            ) {
                changed = true;
                app.actions.push(Action::ProxyEdited);
                proxy_dirty = true;
            }
            ui.add_space(6.0);
            widgets::proxy_scope_note(ui, &palette, app.locale, app.settings.proxy_mode);
            ui.add_space(10.0);
        }
        if app.settings.proxy_mode.is_manual() {
            ui.horizontal(|ui| {
                if theme::pill_button(ui, &palette, &gettext(locale, "Apply settings"), true).clicked() {
                    app.actions.push(Action::ApplyProxy);
                    if app.settings.proxy_config().is_ok() {
                        proxy_dirty = false;
                    }
                }
            });
        } else {
            theme::subtle(
                ui,
                &palette,
                &match app.settings.proxy_mode {
                    ProxyMode::Off => gettext(locale, "Not using a proxy."),
                    ProxyMode::System => gettext(locale, "Using the system proxy."),
                    ProxyMode::Http | ProxyMode::Socks => "".into(),
                },
            );
        }
    });
        });
    }

    let storage = gettext(locale, "Storage");
    let storage_rows = [
        RowText::new(
            gettext(locale, "Artwork cache"),
            // Translators: {folder} is the path of a cache folder.
            gettext(locale, "Stored in {folder}")
                .replace("{folder}", &app.dirs.art_cache_dir().display().to_string()),
        ),
        RowText::new(
            audio_cache.clone(),
            // Translators: {folder} is the path of a cache folder.
            gettext(locale, "Stored in {folder}").replace(
                "{folder}",
                &app.dirs.audio_cache_dir().display().to_string(),
            ),
        ),
        RowText::new(
            gettext(locale, "Play history"),
            gettext(
                locale,
                // Translators: {file} is the path of the play history file.
                "Tracks played here are stored in {file}. This file is never uploaded.",
            )
            .replace("{file}", &app.dirs.history_file().display().to_string()),
        ),
        RowText::new(
            gettext(locale, "Sign-in"),
            gettext(
                locale,
                "Session tokens are stored in owner-only files by melbad. The GUI does not save passwords or verification codes.",
            ),
        ),
    ];
    if section_matches(&needle, &storage, &storage_rows) {
        any_visible = true;
        section(ui, &palette, &storage, |ui| {
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[0], |ui| {
                if theme::soft_button(
                    ui,
                    &palette,
                    Some(Icon::Trash),
                    &gettext(locale, "Clear artwork"),
                    false,
                )
                .clicked()
                {
                    app.actions.push(Action::ClearArtCache);
                }
            });
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[1], |_| {});
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[2], |ui| {
                if theme::soft_button(
                    ui,
                    &palette,
                    Some(Icon::Trash),
                    &gettext(locale, "Clear history"),
                    false,
                )
                .clicked()
                {
                    app.actions.push(Action::ClearPlayHistory);
                }
            });
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[3], |_| {});
        });
    }

    let about = gettext(locale, "About");
    let built_with = gettext(
        locale,
        "Built with Rust and egui on Melba. Not affiliated with Apple.",
    );
    let check_for_updates = gettext(locale, "Check for updates");
    let checking = gettext(locale, "Checking…");
    let keyboard_shortcuts = gettext(locale, "Keyboard shortcuts");
    let source_code = gettext(locale, "Source code");
    let about_rows = [
        RowText::new(
            format!("Melba {}", env!("CARGO_PKG_VERSION")),
            built_with.clone(),
        ),
        RowText::new(
            format!("{check_for_updates} {checking}"),
            format!("{keyboard_shortcuts} {source_code}"),
        ),
    ];
    if section_matches(&needle, &about, &about_rows) {
        any_visible = true;
        section(ui, &palette, &about, |ui| {
            ui.horizontal(|ui| {
                let (logo, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
                theme::logo(ui, logo.center(), 40.0);
                ui.vertical(|ui| {
                    theme::text(
                        ui,
                        format!("Melba {}", env!("CARGO_PKG_VERSION")),
                        theme::semibold(15.0),
                        palette.text,
                    );
                    theme::text(
                        ui,
                        built_with.as_ref(),
                        theme::regular(13.0),
                        palette.secondary,
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let check_label = if app.update_checking {
                    &checking
                } else {
                    &check_for_updates
                };
                if theme::soft_button(ui, &palette, Some(Icon::Refresh), check_label, false)
                    .clicked()
                    && !app.update_checking
                {
                    app.actions.push(Action::CheckForUpdates);
                }
                if theme::soft_button(ui, &palette, Some(Icon::Info), &keyboard_shortcuts, false)
                    .clicked()
                {
                    app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
                }
                if theme::soft_button(ui, &palette, Some(Icon::ExternalLink), &source_code, false)
                    .clicked()
                {
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab(env!("CARGO_PKG_REPOSITORY")));
                }
            });
            ui.add_space(14.0);
            if let Some(url) = widgets::credit(ui, &palette, locale) {
                app.actions.push(Action::OpenUrl(url.to_owned()));
            }
        });
    }

    if !needle.is_empty() && !any_visible {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            // Translators: {query} is the text typed into the settings search field.
            &gettext(locale, "No settings for “{query}”").replace("{query}", &needle),
            &gettext(locale, "Try fewer words, or check the spelling."),
        );
    }

    ui.data_mut(|data| data.insert_temp(dirty_id, playback_dirty));
    ui.data_mut(|data| data.insert_temp(proxy_dirty_id, proxy_dirty));
    if changed {
        app.actions.push(Action::SettingsChanged);
    }
}

/// A band's frequency the short way: 60, 170, 1K, 16K.
/// The interface language: System first, then each language by its own name,
/// so a reader can find theirs whatever language the app is showing.
fn language_picker(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    let system = pgettext(locale, "language", "System");
    let current = app.settings.language;
    let selected = match current {
        LanguageChoice::System => system.clone(),
        LanguageChoice::Locale(chosen) => chosen.native_name().into(),
    };
    let response = egui::ComboBox::from_id_salt("interface_language")
        .selected_text(selected.as_ref())
        .width(200.0_f32.min(ui.available_width()))
        // As many languages as a menu holds before it scrolls, not five.
        .height(1000.0)
        .show_ui(ui, |ui| {
            let choices = std::iter::once((LanguageChoice::System, system.clone())).chain(
                crate::i18n::LOCALES
                    .iter()
                    .map(|&each| (LanguageChoice::Locale(each), each.native_name().into())),
            );
            for (choice, label) in choices {
                if ui
                    .selectable_label(current == choice, label.as_ref())
                    .clicked()
                    && current != choice
                {
                    app.actions.push(Action::SetLanguage(choice));
                }
            }
        });
    let name = gettext(locale, "Language");
    response.response.widget_info(|| {
        let mut info =
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), name.as_ref());
        info.current_text_value = Some(selected.to_string());
        info
    });
}

#[cfg(test)]
mod tests {
    use super::{RowText, row_matches, section_matches};

    #[test]
    fn empty_filter_matches_everything() {
        assert!(row_matches(
            "",
            "Normalize volume",
            "Keep loud tracks level"
        ));
        assert!(section_matches(
            "",
            "Playback on this computer",
            &[RowText::new("Status", "Apple Music")]
        ));
    }

    #[test]
    fn row_matches_title_or_description_case_insensitively() {
        assert!(row_matches(
            "volume",
            "Normalize volume",
            "Keep loud tracks level"
        ));
        assert!(row_matches(
            "VOLUME",
            "Normalize volume",
            "Keep loud tracks level"
        ));
        assert!(row_matches(
            "quiet",
            "Normalize volume",
            "Keep loud and quiet tracks level"
        ));
        assert!(!row_matches(
            "theme",
            "Normalize volume",
            "Keep loud tracks level"
        ));
    }

    #[test]
    fn section_matches_title_or_any_row() {
        let rows = [
            RowText::new("Normalize volume", "Keep loud and quiet tracks level"),
            RowText::new("Autoplay", "Keep playing similar songs"),
        ];
        assert!(section_matches(
            "playback",
            "Playback on this computer",
            &rows
        ));
        assert!(section_matches(
            "autoplay",
            "Playback on this computer",
            &rows
        ));
        assert!(section_matches("quiet", "Playback on this computer", &rows));
        assert!(!section_matches(
            "theme",
            "Playback on this computer",
            &rows
        ));
    }
}
