use gpui::prelude::*;
use gpui::{
    App, Context, Div, Entity, FocusHandle, Global, MouseButton, Render, SharedString, Window, div,
    px, svg,
};
use i18n::t;
use input::{POWERBAR_CONTEXT, PowerbarConfirm, PowerbarNextCategory, PowerbarPrevCategory};
use router::{Destination, navigate};
use state::{Hit, Kind, Origin, Playback, Search};
use ui::{
    ActiveTheme as _, Dismiss, Input, Modal, SelectNext, SelectPrevious, Submit, Text, eyebrow,
};

use crate::shared::tracks::{PlaybackStatus, playback_status};

/// Maximum results shown in the powerbar per category.
const MAX_PER_KIND: usize = 3;

pub(crate) struct Powerbar {
    open: bool,
    input: Entity<Input>,
    search: Entity<Search>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    items: Vec<Hit>,
    selected: Option<usize>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
}

struct Installed(Entity<Powerbar>);
impl Global for Installed {}

impl Powerbar {
    pub fn entity(
        search: Entity<Search>,
        playback: Entity<Playback>,
        cx: &mut App,
    ) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let bar = cx.new(|cx| {
                let current_status = playback_status(&playback, cx);

                let input = cx.new(|cx| {
                    Input::new("powerbar-placeholder", cx)
                        .icon("icons/search.svg")
                        .clearable()
                });
                cx.observe(&input, |this: &mut Powerbar, input, cx| {
                    let query = input.read(cx).text().to_owned();
                    this.search.update(cx, |search, cx| search.ask(&query, cx));
                })
                .detach();

                cx.observe(&search, |this: &mut Powerbar, _, cx| {
                    this.rebuild_items(cx);
                })
                .detach();

                cx.observe(&playback, |this: &mut Powerbar, playback, cx| {
                    let current = playback_status(&playback, cx);
                    if this.playback_status != current {
                        this.playback_status = current;
                        cx.notify();
                    }
                })
                .detach();

                Self {
                    open: false,
                    input,
                    search,
                    playback,
                    playback_status: current_status,
                    items: Vec::new(),
                    selected: None,
                    focus: cx.focus_handle(),
                    restore: None,
                }
            });
            cx.set_global(Installed(bar));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn toggle(window: &mut Window, cx: &mut App) {
        let Some(bar) = cx.try_global::<Installed>().map(|i| i.0.clone()) else {
            return;
        };
        bar.update(cx, |this, cx| {
            if this.open {
                this.close(window, cx);
            } else {
                this.show(window, cx);
            }
        });
    }

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore = window.focused(cx);
        self.open = true;
        self.selected = None;
        self.input.update(cx, |input, cx| input.focus(window, cx));
        let query = self.input.read(cx).text().to_owned();
        self.search.update(cx, |search, cx| search.ask(&query, cx));
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.selected = None;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn rebuild_items(&mut self, cx: &mut Context<Self>) {
        let search = self.search.read(cx);
        if search.query().trim().is_empty() {
            self.items.clear();
            self.selected = None;
            cx.notify();
            return;
        }

        let mut items = Vec::new();
        for kind in Kind::ALL {
            items.extend(search.of(kind).take(MAX_PER_KIND).cloned());
        }
        let len = items.len();
        self.items = items;
        if let Some(sel) = self.selected
            && sel >= len
        {
            self.selected = if len == 0 { None } else { Some(len - 1) };
        }
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        self.selected = Some(match self.selected {
            None => 0,
            Some(i) => (i + 1).min(self.items.len() - 1),
        });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn select_previous(&mut self, _: &SelectPrevious, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected {
            None => {}
            Some(0) => {
                self.selected = None;
                self.input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
            Some(i) => {
                self.selected = Some(i - 1);
                window.focus(&self.focus, cx);
                cx.notify();
            }
        }
    }

    fn category_starts(&self) -> Vec<usize> {
        let mut starts = Vec::new();
        let mut last_kind = None;
        for (i, hit) in self.items.iter().enumerate() {
            let kind = hit.kind();
            if last_kind != Some(kind) {
                starts.push(i);
                last_kind = Some(kind);
            }
        }
        starts
    }

    fn select_next_category(
        &mut self,
        _: &PowerbarNextCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let starts = self.category_starts();
        if starts.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0);
        let next = starts
            .iter()
            .copied()
            .find(|&idx| idx > current)
            .unwrap_or(starts[0]);
        self.selected = Some(next);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn select_prev_category(
        &mut self,
        _: &PowerbarPrevCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let starts = self.category_starts();
        if starts.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0);
        let prev = starts
            .iter()
            .copied()
            .rfind(|&idx| idx < current)
            .unwrap_or(*starts.last().unwrap());
        self.selected = Some(prev);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn activate(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.selected_hit().or_else(|| self.items.first().cloned()) {
            navigate_hit(&hit, cx);
        }
        self.close(window, cx);
    }

    fn play_confirm(&mut self, _: &PowerbarConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.selected_hit().or_else(|| self.items.first().cloned()) {
            play_hit(&hit, &self.playback, cx);
        }
        self.close(window, cx);
    }

    fn selected_hit(&self) -> Option<Hit> {
        self.selected.and_then(|i| self.items.get(i)).cloned()
    }
}

fn category_title(kind: Kind) -> SharedString {
    match kind {
        Kind::Song => t!("search-songs"),
        Kind::Artist => t!("search-artists"),
        Kind::Album => t!("nav-albums"),
        Kind::Playlist => t!("nav-playlists"),
    }
}

fn navigate_hit(hit: &Hit, cx: &mut App) {
    match hit {
        Hit::Song(track) => {
            if let Some(id) = &track.id {
                navigate(Destination::Song(SharedString::from(id.clone())), cx);
            }
        }
        Hit::Artist(artist) => {
            if let Some(id) = &artist.id {
                navigate(Destination::Artist(SharedString::from(id.clone())), cx);
            }
        }
        Hit::Album(album) => {
            navigate(Destination::Album(SharedString::from(album.id.clone())), cx);
        }
        Hit::Playlist(list) => {
            navigate(
                Destination::Playlist(SharedString::from(list.id.clone())),
                cx,
            );
        }
    }
}

fn play_hit(hit: &Hit, playback: &Entity<Playback>, cx: &mut App) {
    playback.update(cx, |playback, cx| match hit {
        Hit::Song(track) => playback.play_radio(track, cx),
        Hit::Artist(artist) => {
            if let Some(id) = &artist.id {
                playback.play_origin(Origin::artist(id.clone()).named(artist.name.clone()), cx);
            }
        }
        Hit::Album(album) => {
            playback.play_origin(
                Origin::album(album.id.clone()).named(album.name.clone()),
                cx,
            );
        }
        Hit::Playlist(list) => {
            playback.play_origin(
                Origin::playlist(list.id.clone()).named(list.name.clone()),
                cx,
            );
        }
    });
}

impl Render for Powerbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        let theme = *cx.theme();
        let items = self.items.clone();
        let selected = self.selected;

        let mut grouped: Vec<(Kind, Vec<(usize, Hit)>)> = Vec::new();
        for (i, hit) in items.iter().enumerate() {
            let kind = hit.kind();
            if let Some(group) = grouped.iter_mut().find(|(k, _)| *k == kind) {
                group.1.push((i, hit.clone()));
            } else {
                grouped.push((kind, vec![(i, hit.clone())]));
            }
        }

        div()
            .absolute()
            .inset_0()
            .key_context(POWERBAR_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next_category))
            .on_action(cx.listener(Self::select_prev_category))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::play_confirm))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .child(
                Modal::new("powerbar", t!("powerbar-title"))
                    .w(theme.metrics.cover * 5.0)
                    .child(self.input.clone())
                    .when(!grouped.is_empty(), |modal| {
                        let total_groups = grouped.len();
                        modal.child(
                            div().flex().flex_col().pt_1().children(
                                grouped.into_iter().enumerate().map(
                                    |(g_idx, (kind, group_items))| {
                                        let is_last = g_idx + 1 == total_groups;
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(2.))
                                            .py_2()
                                            .when(!is_last, |this| {
                                                this.border_b_1().border_color(theme.border)
                                            })
                                            .child(
                                                div()
                                                    .px_2()
                                                    .pb_1()
                                                    .child(eyebrow(category_title(kind), cx)),
                                            )
                                            .children(group_items.into_iter().map(|(idx, hit)| {
                                                let is_chosen = selected == Some(idx);
                                                hit_row(&hit, is_chosen, &theme).on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(move |this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        navigate_hit(&hit, cx);
                                                        this.close(window, cx);
                                                    }),
                                                )
                                            }))
                                    },
                                ),
                            ),
                        )
                    })
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

fn hit_row(hit: &Hit, chosen: bool, theme: &ui::Theme) -> Div {
    let (label, subtitle, cover, icon) = describe_hit(hit);

    let bg = match chosen {
        true => theme.secondary,
        false => theme.background,
    };

    div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .cursor_pointer()
        .bg(bg)
        .hover(|style| match chosen {
            true => style,
            false => style.bg(theme.secondary_hover),
        })
        .child(
            div()
                .flex_none()
                .size(px(36.))
                .rounded(theme.radius)
                .overflow_hidden()
                .bg(theme.secondary)
                .map(|d| match cover {
                    Some(url) => d.child(
                        gpui::img(url)
                            .size_full()
                            .object_fit(gpui::ObjectFit::Cover),
                    ),
                    None => d.flex().items_center().justify_center().child(
                        svg()
                            .path(icons::path(icon))
                            .size(px(18.))
                            .text_color(theme.muted_foreground),
                    ),
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(theme.text(Text::Body))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(label),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme.text(Text::Small))
                        .text_color(theme.muted_foreground)
                        .child(subtitle),
                ),
        )
}

fn describe_hit(hit: &Hit) -> (SharedString, SharedString, Option<String>, &'static str) {
    match hit {
        Hit::Song(track) => (
            SharedString::from(track.name.clone()),
            SharedString::from(format!("{} · {}", track.artists, track.album)),
            track.cover.clone(),
            "icons/music.svg",
        ),
        Hit::Artist(artist) => (
            SharedString::from(artist.name.clone()),
            t!("artist-eyebrow"),
            artist.cover.clone(),
            "icons/user.svg",
        ),
        Hit::Album(album) => (
            SharedString::from(album.name.clone()),
            SharedString::from(album.artists.clone()),
            album.cover.clone(),
            "icons/disc-3.svg",
        ),
        Hit::Playlist(list) => (
            SharedString::from(list.name.clone()),
            SharedString::from(list.owner.clone()),
            list.cover.clone(),
            "icons/list.svg",
        ),
    }
}
