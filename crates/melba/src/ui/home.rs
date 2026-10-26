//! The Home page.

use std::sync::Arc;

use egui::{CornerRadius, Rect, Sense, Vec2, pos2, vec2};

use crate::api::models::{PlayableItem, Playlist, pick_image};
use crate::app::App;
use crate::i18n::gettext;
use crate::model::{Action, Loadable, Page, RowContext};
use crate::theme::{self, Icon};

use super::widgets::{self, TrackRow};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    if !app.network_reachable {
        let space = ((ui.available_height() - 160.0) / 2.0).max(48.0);
        ui.add_space(space);
        ui.vertical_centered(|ui| {
            theme::icon(ui, Icon::WifiOff, 54.0, palette.dim);
            ui.add_space(16.0);
            theme::text(
                ui,
                gettext(app.locale, "You're offline"),
                theme::bold(28.0),
                palette.text,
            );
            ui.add_space(8.0);
            theme::text(
                ui,
                gettext(
                    app.locale,
                    "Melba will reconnect automatically when your connection returns.",
                ),
                theme::regular(14.0),
                palette.secondary,
            );
        });
        return;
    }
    ui.add_space(6.0);
    let greeting = crate::util::greeting(app.locale);
    theme::text(ui, greeting.as_ref(), theme::bold(30.0), palette.text);
    ui.add_space(12.0);
    quick_access(app, ui);
    ui.add_space(16.0);

    if app.settings.home.made_for_you.visible {
        made_for_you(app, ui);
    }
    recently_played(app, ui);
    top_artists(app, ui);
    top_tracks(app, ui);
    if app.settings.home.recommendations.visible {
        recommendations(app, ui);
    }
}

struct Tile {
    image: Option<String>,
    name: String,
    page: Page,
    uri: Option<String>,
    liked: bool,
    owned_playlist: Option<Playlist>,
}

/// The height of a quick-access tile, and so the side of its cover.
const TILE_HEIGHT: f32 = 60.0;

/// Liked Songs and the first playlists, as Home's quick-access tiles.
fn quick_access_tiles(app: &App) -> Vec<Tile> {
    let tile_art = app.art_px(TILE_HEIGHT);
    let mut tiles: Vec<Tile> = vec![Tile {
        image: None,
        name: gettext(app.locale, "Liked Songs").into_owned(),
        page: Page::LikedSongs,
        uri: app
            .user
            .as_ref()
            .map(|user| format!("melba:user:{}:collection", user.id)),
        liked: true,
        owned_playlist: None,
    }];
    if let Some(playlists) = app.library.playlists.get() {
        for playlist in playlists.iter().take(7) {
            tiles.push(Tile {
                image: pick_image(&playlist.images, tile_art).map(str::to_string),
                name: playlist.name.clone(),
                page: Page::Playlist(playlist.id.clone()),
                uri: Some(playlist.uri.clone()),
                liked: false,
                owned_playlist: app
                    .user_id()
                    .is_some_and(|id| playlist.owned_by(id))
                    .then(|| playlist.clone()),
            });
        }
    }
    tiles
}

fn quick_access(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let tiles = quick_access_tiles(app);
    let available = ui.available_width();
    let columns = ((available / 300.0).floor() as usize).clamp(2, 4);
    let gap = 10.0;
    let tile_width = (available - gap * (columns as f32 - 1.0)) / columns as f32;
    let rows = tiles.len().div_ceil(columns);
    for row in 0..rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for column in 0..columns {
                let Some(Tile {
                    image,
                    name,
                    page,
                    uri,
                    liked,
                    owned_playlist,
                }) = tiles.get(row * columns + column)
                else {
                    break;
                };
                let (rect, response) =
                    ui.allocate_exact_size(vec2(tile_width, TILE_HEIGHT), Sense::click());
                if ui.is_rect_visible(rect) {
                    let hovered = ui.rect_contains_pointer(rect);
                    let fill = if hovered {
                        palette.surface_hover
                    } else {
                        palette.surface
                    };
                    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
                    let cover = Rect::from_min_size(rect.min, Vec2::splat(TILE_HEIGHT));
                    if *liked {
                        super::sidebar::liked_cover(ui, cover, 6.0);
                    } else {
                        widgets::paint_cover(
                            ui,
                            &palette,
                            image.as_deref(),
                            cover,
                            6.0,
                            Icon::Music,
                            Some(app.backend.art()),
                        );
                    }
                    let play_room = if hovered && uri.is_some() { 52.0 } else { 12.0 };
                    let text_rect = Rect::from_min_max(
                        pos2(cover.right() + 12.0, rect.top()),
                        pos2(rect.right() - play_room, rect.bottom()),
                    );
                    crate::bidi::paint_line(
                        &ui.painter().with_clip_rect(text_rect),
                        text_rect.left(),
                        text_rect.right(),
                        rect.center().y,
                        name,
                        theme::bold(14.5),
                        palette.text,
                    );
                    if hovered && let Some(uri) = uri {
                        let context_here =
                            app.playing_context_uri().as_deref() == Some(uri.as_str());
                        let playing_here = context_here && app.believed_playing();
                        let button = Rect::from_center_size(
                            pos2(rect.right() - 28.0, rect.center().y),
                            Vec2::splat(40.0),
                        );
                        let mut child =
                            ui.new_child(egui::UiBuilder::new().max_rect(button).layout(
                                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                            ));
                        if theme::circle_button(
                            &mut child,
                            if playing_here {
                                Icon::PauseFilled
                            } else {
                                Icon::PlayFilled
                            },
                            40.0,
                            palette.accent,
                            palette.accent_hover,
                            palette.on_accent,
                            &gettext(app.locale, if playing_here { "Pause" } else { "Play" }),
                        )
                        .clicked()
                        {
                            if context_here {
                                app.actions.push(Action::TogglePlay);
                            } else {
                                app.actions.push(Action::PlayContext {
                                    uri: uri.clone(),
                                    offset_uri: None,
                                    offset_index: None,
                                });
                            }
                        }
                    }
                }
                if response.clicked() {
                    app.actions.push(Action::Open(page.clone()));
                }
                if !liked && let Some(uri) = uri {
                    egui::Popup::context_menu(&response)
                        .id(ui.make_persistent_id(("quick-access-menu", uri)))
                        .frame(widgets::menu_frame(&palette))
                        .show(|ui| {
                            widgets::context_menu_items(
                                ui,
                                app,
                                uri,
                                name,
                                owned_playlist.as_ref(),
                            );
                        });
                }
            }
        });
    }
}

/// Renders session Home cards or approximate fallback cards in the same shelf.
fn made_for_you(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let (playlists, loading, failure) = match &app.home.made_for_you {
        Loadable::Loaded(playlists) => (playlists.clone(), false, None),
        Loadable::Loading | Loadable::NotLoaded => (Vec::new(), true, None),
        Loadable::Failed(error) => (Vec::new(), false, Some(error.clone())),
    };
    if playlists.is_empty() && !loading && failure.is_none() {
        return;
    }
    widgets::shelf(
        ui,
        &palette,
        "made-for-you",
        &gettext(app.locale, "Made for you"),
        |ui| {
            if playlists.is_empty() && loading {
                widgets::loading_row(ui, &palette, app.locale);
            } else if playlists.is_empty()
                && let Some(message) = &failure
            {
                widgets::error_row(ui, app, message, Some(Page::Home));
            }
            for playlist in &playlists {
                let subtitle = playlist
                    .description
                    .as_deref()
                    .map(crate::util::strip_html)
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| {
                        // Translators: {owner} is the name of the playlist's owner.
                        gettext(app.locale, "By {owner}").replace("{owner}", playlist.owner_name())
                    });
                let context_here =
                    app.playing_context_uri().as_deref() == Some(playlist.uri.as_str());
                let playing_here = context_here && app.believed_playing();
                let card_art = app.art_px(widgets::CARD_ART);
                let card = widgets::card(
                    ui,
                    app,
                    pick_image(&playlist.images, card_art),
                    &playlist.name,
                    &subtitle,
                    widgets::CardCover::square(playing_here),
                );
                if card.play {
                    if context_here {
                        app.actions.push(Action::TogglePlay);
                    } else {
                        app.actions.push(Action::PlayContext {
                            uri: playlist.uri.clone(),
                            offset_uri: None,
                            offset_index: None,
                        });
                    }
                }
                if card.clicked {
                    app.actions
                        .push(Action::Open(Page::Playlist(playlist.id.clone())));
                }
                egui::Popup::context_menu(&card.response)
                    .id(ui.make_persistent_id(("home-made_for_you-menu", &playlist.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        let owned = app.user_id().is_some_and(|id| playlist.owned_by(id));
                        widgets::context_menu_items(
                            ui,
                            app,
                            &playlist.uri,
                            &playlist.name,
                            owned.then_some(playlist),
                        );
                    });
            }
        },
    );
}

fn recently_played(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let history = match app.home.recently_played.clone() {
        Loadable::Loaded(history) => history,
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::shelf(
                ui,
                &palette,
                "recent",
                &gettext(app.locale, "Recently played"),
                |ui| widgets::loading_row(ui, &palette, app.locale),
            );
            return;
        }
        Loadable::Failed(message) => {
            widgets::shelf(
                ui,
                &palette,
                "recent",
                &gettext(app.locale, "Recently played"),
                |ui| {
                    widgets::error_row(ui, app, &message, Some(Page::Home));
                },
            );
            return;
        }
    };
    let mut seen = std::collections::HashSet::new();
    let tracks: Vec<_> = history
        .into_iter()
        .filter(|entry| {
            entry
                .track
                .id
                .as_ref()
                .is_some_and(|id| seen.insert(id.clone()))
        })
        .take(16)
        .collect();
    if tracks.is_empty() {
        return;
    }
    widgets::shelf(
        ui,
        &palette,
        "recent",
        &gettext(app.locale, "Recently played"),
        |ui| {
            for entry in &tracks {
                let track = &entry.track;
                let card_art = app.art_px(widgets::CARD_ART);
                let card = widgets::card(
                    ui,
                    app,
                    track.image(card_art),
                    &track.name,
                    &track.artist_names(),
                    widgets::CardCover::square(false),
                );
                if card.play {
                    app.actions.push(Action::PlayUris {
                        uris: vec![track.uri.clone()],
                        index: 0,
                    });
                }
                if card.clicked
                    && let Some(album) = &track.album
                    && !album.id.is_empty()
                {
                    app.actions
                        .push(Action::Open(Page::Album(album.id.clone())));
                }
                egui::Popup::context_menu(&card.response)
                    .id(ui.make_persistent_id(("home-recently_played-menu", &track.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        widgets::item_menu(
                            ui,
                            app,
                            &PlayableItem::Track(track.clone()),
                            None,
                            None,
                        );
                    });
            }
        },
    );
}

fn top_artists(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let artists = match app.home.top_artists.clone() {
        Loadable::Loaded(artists) => artists,
        Loadable::Loading | Loadable::NotLoaded => {
            widgets::shelf(
                ui,
                &palette,
                "top-artists",
                &gettext(app.locale, "Your top artists"),
                |ui| widgets::loading_row(ui, &palette, app.locale),
            );
            return;
        }
        Loadable::Failed(message) => {
            widgets::shelf(
                ui,
                &palette,
                "top-artists",
                &gettext(app.locale, "Your top artists"),
                |ui| {
                    widgets::error_row(ui, app, &message, Some(Page::Home));
                },
            );
            return;
        }
    };
    if artists.is_empty() {
        return;
    }
    widgets::shelf(
        ui,
        &palette,
        "top-artists",
        &gettext(app.locale, "Your top artists"),
        |ui| {
            for artist in &artists {
                let context_here =
                    app.playing_context_uri().as_deref() == Some(artist.uri.as_str());
                let playing_here = context_here && app.believed_playing();
                let card_art = app.art_px(widgets::CARD_ART);
                let card = widgets::card(
                    ui,
                    app,
                    pick_image(&artist.images, card_art),
                    &artist.name,
                    &gettext(app.locale, "Artist"),
                    widgets::CardCover::portrait(playing_here),
                );
                if card.play {
                    if context_here {
                        app.actions.push(Action::TogglePlay);
                    } else {
                        app.actions.push(Action::PlayContext {
                            uri: artist.uri.clone(),
                            offset_uri: None,
                            offset_index: None,
                        });
                    }
                }
                if card.clicked {
                    app.actions
                        .push(Action::Open(Page::Artist(artist.id.clone())));
                }
                egui::Popup::context_menu(&card.response)
                    .id(ui.make_persistent_id(("home-top_artists-menu", &artist.uri)))
                    .frame(widgets::menu_frame(&palette))
                    .show(|ui| {
                        widgets::context_menu_items(ui, app, &artist.uri, &artist.name, None);
                    });
            }
        },
    );
}

fn track_list(
    app: &mut App,
    ui: &mut egui::Ui,
    title: &str,
    tracks: Loadable<Vec<crate::api::models::Track>>,
    limit: usize,
    title_page: Option<Page>,
    more_label: Option<&str>,
) {
    let palette = app.palette;
    let tracks = match tracks {
        Loadable::Loaded(tracks) => tracks,
        Loadable::Loading | Loadable::NotLoaded => {
            if let Some(page) = title_page {
                if theme::link(ui, title, theme::bold(17.0), palette.text).clicked() {
                    app.actions.push(Action::Open(page));
                }
            } else {
                theme::section_title(ui, &palette, title);
            }
            widgets::loading_row(ui, &palette, app.locale);
            ui.add_space(12.0);
            return;
        }
        Loadable::Failed(message) => {
            theme::section_title(ui, &palette, title);
            widgets::error_row(ui, app, &message, Some(title_page.unwrap_or(Page::Home)));
            ui.add_space(12.0);
            return;
        }
    };
    if tracks.is_empty() {
        return;
    }
    if let Some(page) = title_page {
        if theme::link(ui, title, theme::bold(17.0), palette.text).clicked() {
            app.actions.push(Action::Open(page));
        }
    } else {
        theme::section_title(ui, &palette, title);
    }
    ui.add_space(4.0);
    let uris: Arc<[String]> = tracks
        .iter()
        .map(|track| track.uri.clone())
        .collect::<Vec<_>>()
        .into();
    let context = RowContext::Uris(Arc::clone(&uris));
    for (index, track) in tracks.iter().take(limit).enumerate() {
        let item = PlayableItem::Track(track.clone());
        widgets::track_row(
            ui,
            app,
            TrackRow {
                index,
                number: None,
                item: &item,
                context: &context,
                show_cover: !app.settings.tracklist_compact,
                show_album: true,
                added_at: None,
                compact: false,
                thin: app.settings.tracklist_compact,
                shift: 0.0,
                picked: false,
                picked_songs: &[],
            },
        );
    }
    if let Some(label) = more_label
        && tracks.len() > limit
        && theme::link(ui, label, theme::semibold(14.0), palette.secondary).clicked()
    {
        app.actions.push(Action::Open(Page::TopSongs));
    }
    ui.add_space(16.0);
}

fn top_tracks(app: &mut App, ui: &mut egui::Ui) {
    let tracks = app.home.top_tracks.clone();
    let title = gettext(app.locale, "Your top songs");
    let more = gettext(app.locale, "Show more top songs");
    track_list(
        app,
        ui,
        &title,
        tracks,
        10,
        Some(Page::TopSongs),
        Some(&more),
    );
}

fn recommendations(app: &mut App, ui: &mut egui::Ui) {
    let tracks = app.home.recommendations.clone();
    let title = gettext(app.locale, "Recommended for you");
    track_list(app, ui, &title, tracks, 20, None, None);
}

#[cfg(all(test, feature = "demo"))]
mod tests {
    use super::*;

    /// A tile's cover is 60 points. Spotify's 64 pixel rendition would be
    /// blurry even at 100%, so the tile takes the 300 pixel one and scales
    /// it down; at 300% the 300 pixel one still has a quarter more pixels
    /// than the tile shows.
    #[cfg(feature = "demo")]
    #[test]
    fn quick_access_tiles_ask_for_art_the_display_can_show() {
        let root = std::env::temp_dir().join(format!("melba-tiles-{}", std::process::id()));
        let mut app = App::new(
            &crate::backend::Waker::default(),
            crate::paths::AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            crate::settings::Settings::default(),
            crate::app::AppOptions {
                media_controls: false,
                restore_sign_in: false,
                tray: false,
            },
        );
        crate::demo::populate(&mut app);
        let first = app.library.playlists.get().unwrap()[0].images.clone();
        let width = |url: Option<&str>| {
            first
                .iter()
                .find(|image| Some(image.url.as_str()) == url)
                .and_then(|image| image.width)
        };
        app.pixels_per_point = 1.0;
        assert_eq!(
            width(quick_access_tiles(&app)[1].image.as_deref()),
            Some(300)
        );
        app.pixels_per_point = 3.0;
        assert_eq!(
            width(quick_access_tiles(&app)[1].image.as_deref()),
            Some(300)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
