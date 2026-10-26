//! The sign-in screen.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Rect, Stroke, Vec2, pos2};

use crate::app::App;
use crate::backend::AuthStatus;
use crate::i18n::gettext;
use crate::model::Action;
use crate::settings::ProxyMode;
use crate::theme;

/// Width of the login card's scroll bar lane.
const SCROLL_BAR_LANE: f32 = 16.0;
/// Inner margin between the card's edge and its scrolling content.
const CARD_MARGIN: f32 = 36.0;

/// Card geometry, shared by the view and its layout regression: a short
/// window shrinks the header rather than pushing the prompt's own controls
/// past the card's edge.
pub(crate) fn card_layout(
    rect: Rect,
    auth: &AuthStatus,
    proxy_open: bool,
    manual_proxy: bool,
) -> (Rect, bool) {
    let compact = rect.height() < 640.0;
    let width = (rect.width() - 32.0).clamp(0.0, 440.0);
    let height: f32 = if matches!(auth, AuthStatus::Prompt { .. }) {
        620.0
    } else if !proxy_open {
        400.0
    } else if manual_proxy {
        720.0
    } else {
        500.0
    };
    let height = height.min((rect.height() - 48.0).max(0.0));
    (
        Rect::from_center_size(
            rect.center() - Vec2::new(0.0, 20.0),
            Vec2::new(width, height),
        ),
        compact,
    )
}

pub fn show(app: &mut App, ui: &mut egui::Ui, connecting: bool) {
    let palette = app.palette;
    let locale = app.locale;
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            // The native double-click action belongs to the top bar, not the empty login background.
            let drag_rect = if cfg!(target_os = "macos") {
                Rect::from_min_size(
                    rect.min,
                    Vec2::new(rect.width(), theme::TOP_BAR_HEIGHT + theme::titlebar_inset(ui.ctx())),
                )
            } else {
                rect
            };
            fastframe_titlebar::drag(ui, drag_rect);
            let top = super::blend(palette.window, palette.accent, 0.10);
            super::widgets::paint_vertical_gradient(ui, rect, top, palette.window);
            let proxy_id = egui::Id::new("login-proxy-open");
            let proxy_open = ui
                .ctx()
                .data(|data| data.get_temp::<bool>(proxy_id))
                .unwrap_or(false);
            let (card, compact) = card_layout(
                rect,
                &app.auth,
                proxy_open,
                app.settings.proxy_mode.is_manual(),
            );
            let card_width = card.width();
            let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(card).layout(Layout::top_down(Align::Center)));
            Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 8))
                .inner_margin(Margin::same(CARD_MARGIN as i8))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 16],
                    blur: 48,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(&mut card_ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("login-card-scroll")
                        .max_height((card.height() - 2.0 * CARD_MARGIN).max(0.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                    // Leave the scroll bar its own lane, otherwise a wrapped
                    // message runs underneath it.
                    ui.set_width((card_width - 2.0 * CARD_MARGIN - SCROLL_BAR_LANE).max(0.0));
                    ui.spacing_mut().item_spacing.y = if compact { 4.0 } else { 8.0 };
                    if compact {
                        // Short windows keep every control on screen; the
                        // prompt's own fields and choices are what matter.
                        ui.spacing_mut().interact_size.y = 24.0;
                    }
                    let logo_size = if compact { 32.0 } else { 72.0 };
                    let (logo, _) =
                        ui.allocate_exact_size(Vec2::splat(logo_size), egui::Sense::hover());
                    theme::logo(ui, logo.center(), logo_size);
                    ui.add_space(if compact { 2.0 } else { 6.0 });
                    theme::text(ui, "Melba", theme::bold(if compact { 22.0 } else { 30.0 }), palette.text);
                    if !compact {
                        theme::text(ui, gettext(locale, "A native Apple Music client."), theme::regular(14.5), palette.secondary);
                    }
                    ui.add_space(if compact { 10.0 } else { 22.0 });
                    match &app.auth {
                        AuthStatus::WaitingForBrowser { url } => {
                            let url = url.clone();
                            ui.horizontal(|ui| {
                                ui.add_space((ui.available_width() - 250.0).max(0.0) / 2.0);
                                theme::spinner(ui, 18.0, palette.accent);
                                theme::text(ui, gettext(locale, "Waiting for Apple Music in your browser…"), theme::medium(14.0), palette.text);
                            });
                            ui.add_space(6.0);
                            if theme::link(ui, gettext(locale, "Didn't open? Open the sign-in page again"), theme::regular(13.0), palette.secondary).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(url));
                            }
                            ui.add_space(14.0);
                            if theme::pill_button(ui, &palette, &gettext(locale, "Cancel"), false).clicked() {
                                app.actions.push(Action::CancelSignIn);
                            }
                        }
                        AuthStatus::Prompt { attempt, prompt } => {
                            let (attempt, prompt) = (*attempt, prompt.clone());
                            auth_prompt(ui, app, attempt, &prompt);
                        }
                        _ if connecting => {
                            ui.horizontal(|ui| {
                                ui.add_space((ui.available_width() - 200.0).max(0.0) / 2.0);
                                theme::spinner(ui, 18.0, palette.accent);
                                theme::text(ui, gettext(locale, "Connecting to Apple Music…"), theme::medium(14.0), palette.text);
                            });
                            if theme::pill_button(ui, &palette, &gettext(locale, "Cancel"), false).clicked() {
                                app.actions.push(Action::CancelSignIn);
                            }
                        }
                        AuthStatus::Failed(message) => {
                            let message = message.clone();
                            ui.add(
                                egui::Label::new(egui::RichText::new(message).font(theme::regular(13.0)).color(palette.danger)).wrap(),
                            );
                            ui.add_space(12.0);
                            if big_button(ui, app, &gettext(locale, "Try again")) {
                                app.actions.push(Action::SignIn);
                            }
                                        }
                        _ => {
                            if big_button(ui, app, &gettext(locale, "Sign in with Apple Music")) {
                                app.actions.push(Action::SignIn);
                            }
                            ui.add_space(10.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(gettext(locale, "Use your browser, or sign in locally in Melba when the managed wrapper is installed. Credentials are not saved by the GUI. Playback needs an Apple Music subscription."))
                                        .font(theme::regular(12.5))
                                        .color(palette.secondary),
                                )
                                .wrap(),
                            );
                        }
                    }
                    let mut proxy_open = ui.data(|data| data.get_temp::<bool>(proxy_id)).unwrap_or(false);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::top_down(Align::Center), |ui| {
                            if theme::link(
                                ui,
                                gettext(locale, "Proxy Settings"),
                                theme::regular(13.0),
                                palette.secondary,
                            )
                            .clicked()
                            {
                                proxy_open = !proxy_open;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    if proxy_open {
                        proxy_fields(ui, app);
                    }
                    ui.data_mut(|data| data.insert_temp(proxy_id, proxy_open));
                        });
                });
            ui.painter().text(
                pos2(rect.center().x, rect.bottom() - 24.0),
                egui::Align2::CENTER_BOTTOM,
                // Translators: {version} is the app's version number, such as 1.2.0.
                gettext(locale, "Melba {version} • not affiliated with Apple")
                    .replace("{version}", env!("CARGO_PKG_VERSION")),
                theme::regular(11.5),
                palette.dim,
            );
        });
}

fn auth_field(
    ui: &mut egui::Ui,
    value: &mut melba_auth::Secret,
    id: egui::Id,
    password: bool,
    limit: usize,
) -> egui::Response {
    let mut field = egui::TextEdit::singleline(&mut **value)
        .id(id)
        .password(password)
        .char_limit(limit)
        .desired_width(f32::INFINITY)
        .show(ui);
    // TextEditState persists its cursor, not its text, but its RAM undoer can
    // retain credential copies across prompts. Do not keep auth undo history.
    field.state.set_undoer(Default::default());
    field.state.store(ui.ctx(), id);
    field.response.response
}

fn auth_prompt(ui: &mut egui::Ui, app: &mut App, attempt: u64, prompt: &melba_auth::Prompt) {
    use melba_auth::PromptKind;
    ui.add(egui::Label::new(egui::RichText::new(&prompt.title).strong()).wrap());
    ui.add(egui::Label::new(&prompt.message).wrap());
    let mut form = app.auth_form.clone();
    let mut changed = false;
    let mut submit = false;
    let mut valid = prompt.kind != PromptKind::Dialog;
    if matches!(
        prompt.kind,
        PromptKind::Credentials | PromptKind::CredentialsAndCode
    ) {
        ui.label("Apple ID");
        changed |= auth_field(
            ui,
            &mut form.username,
            egui::Id::new(("auth-user", attempt, prompt.id)),
            false,
            1023,
        )
        .changed();
        ui.label(gettext(app.locale, "Password"));
        let password = auth_field(
            ui,
            &mut form.password,
            egui::Id::new(("auth-password", attempt, prompt.id)),
            true,
            4095,
        );
        changed |= password.changed();
        if prompt.kind == PromptKind::Credentials {
            submit = password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }
        valid &= !form.username.is_empty() && !form.password.is_empty();
    }
    if matches!(
        prompt.kind,
        PromptKind::Code | PromptKind::CredentialsAndCode
    ) {
        ui.label("Verification code");
        let code = auth_field(
            ui,
            &mut form.code,
            egui::Id::new(("auth-code", attempt, prompt.id)),
            true,
            6,
        );
        changed |= code.changed();
        submit = code.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        valid &= form.code.len() == 6 && form.code.bytes().all(|c| c.is_ascii_digit());
    }
    if changed {
        app.actions.push(Action::AuthForm {
            attempt,
            id: prompt.id,
            form: Box::new(form),
        });
    }
    if prompt.kind != PromptKind::Dialog {
        let label = prompt
            .buttons
            .iter()
            .find(|b| b.id == 2)
            .map(|b| b.label.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Continue");
        if ui
            .add_enabled(valid, egui::Button::new(label).wrap())
            .clicked()
            || (submit && valid)
        {
            app.actions.push(Action::SubmitAuth {
                attempt,
                id: prompt.id,
            });
        }
    }
    // Preserve the actual SDK buttons and ordinal. Do not invent a local
    // SMS/phone selector or claim a delivery request from a selection alone.
    for button in &prompt.buttons {
        if prompt.kind != PromptKind::Dialog && button.id == 2 {
            continue;
        }
        if !button.label.is_empty() && ui.add(egui::Button::new(&button.label).wrap()).clicked() {
            app.actions.push(Action::SelectAuth {
                attempt,
                id: prompt.id,
                button: button.id,
            });
        }
    }
    ui.add_space(8.0);
    if ui.button(gettext(app.locale, "Cancel sign-in")).clicked() {
        app.actions.push(Action::CancelSignIn);
    }
}

fn proxy_fields(ui: &mut egui::Ui, app: &mut App) {
    let palette = app.palette;
    let mut changed = false;
    let mut apply = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let row_width: f32 = ProxyMode::ALL
            .iter()
            .map(|choice| {
                let galley = ui.painter().layout_no_wrap(
                    choice.label(app.locale).into_owned(),
                    theme::medium(13.0),
                    palette.text,
                );
                galley.size().x + 24.0
            })
            .sum::<f32>()
            + 6.0 * (ProxyMode::ALL.len() - 1) as f32;
        ui.add_space((ui.available_width() - row_width).max(0.0) / 2.0);
        for choice in ProxyMode::ALL {
            if theme::soft_button(
                ui,
                &palette,
                None,
                &choice.label(app.locale),
                app.settings.proxy_mode == choice,
            )
            .clicked()
                && app.settings.proxy_mode != choice
            {
                app.settings.proxy_mode = choice;
                changed = true;
                apply = !choice.is_manual();
            }
        }
    });
    if app.settings.proxy_mode.is_manual() {
        ui.add_space(10.0);
        if super::widgets::proxy_manual_form(
            ui,
            &palette,
            app.locale,
            &mut app.settings.proxy_host,
            &mut app.settings.proxy_port,
            &mut app.settings.proxy_username,
            &mut app.settings.proxy_password,
        ) {
            changed = true;
        }
        ui.add_space(6.0);
        super::widgets::proxy_scope_note(ui, &palette, app.locale, app.settings.proxy_mode);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if theme::pill_button(ui, &palette, &gettext(app.locale, "Apply settings"), true)
                .clicked()
            {
                apply = true;
            }
        });
    }
    if changed {
        app.actions.push(Action::ProxyEdited);
        app.mark_settings_dirty();
    }
    if apply {
        app.actions.push(Action::ApplyProxy);
    }
}

fn big_button(ui: &mut egui::Ui, app: &App, label: &str) -> bool {
    let palette = app.palette;
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), theme::bold(15.0), palette.on_accent);
    let size = Vec2::new(ui.available_width().min(300.0), 46.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let fill = if response.hovered() {
        palette.accent_hover
    } else {
        palette.accent
    };
    ui.painter().rect_filled(rect, 23.0, fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        palette.on_accent,
    );
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::AppOptions, backend::Waker, paths::AppDirs, settings::Settings};

    #[test]
    fn expanded_proxy_settings_remain_reachable_in_a_short_window() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install(&ctx);
        let root =
            std::env::temp_dir().join(format!("melba-proxy-short-login-{}", std::process::id()));
        let mut app = App::new(
            &Waker::default(),
            AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            Settings {
                proxy_mode: ProxyMode::Http,
                ..Default::default()
            },
            AppOptions {
                media_controls: false,
                restore_sign_in: false,
                tray: false,
            },
        );
        app.auth = AuthStatus::SignedOut;
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("login-proxy-open"), true));
        let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(760.0, 520.0));
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| show(&mut app, ui, false),
            );
            output.textures_delta.clear();
            output.platform_output.accesskit_update.unwrap()
        };
        let visible_button = |tree: &egui::accesskit::TreeUpdate, label: &str| {
            let bounds = tree
                .nodes
                .iter()
                .find(|(_, node)| {
                    node.label() == Some(label) && node.role() == egui::accesskit::Role::Button
                })
                .unwrap_or_else(|| panic!("missing accessible {label}"))
                .1
                .bounds()
                .unwrap();
            bounds.y0 >= 0.0 && bounds.y1 <= 480.0
        };
        let first = frame(vec![]);
        assert!(visible_button(&first, "Sign in with Apple Music"));
        let center = |label: &str| {
            let bounds = first
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label))
                .unwrap_or_else(|| panic!("missing accessible {label}"))
                .1
                .bounds()
                .unwrap();
            (bounds.x0 + bounds.x1) / 2.0
        };
        assert!(
            (center("Proxy Settings") - center("Sign in with Apple Music")).abs() < 1.0,
            "the proxy link stays centered under sign-in"
        );
        frame(vec![
            egui::Event::PointerMoved(egui::pos2(400.0, 250.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, -1000.0),
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        for _ in 0..30 {
            frame(vec![]);
        }
        assert!(
            visible_button(&frame(vec![]), "Apply settings"),
            "scrolling must reach Apply above the footer"
        );
    }
}
