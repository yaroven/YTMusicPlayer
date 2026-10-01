//! Slint UI, styled after YouTube Music: always dark, red accent, two-line
//! track rows with album art, a player bar with a full-width progress line
//! and a full-screen "now playing" view. Responsive by window width:
//! - < 1050 px: narrower sidebar, fewer player-bar buttons;
//! - < 820 px: no sidebar (playlist picker under the title);
//! - < 620 px: icon-only buttons, no time / modes in the bar.
//!
//! The sidebar is resized by dragging its right edge (double-click resets).
//! The search field filters the list as you type; Enter searches YouTube
//! Music.
//!
//! Declarative only; behaviour lives in `gui::run`.

slint::slint! {
    import { ListView, AboutSlint } from "std-widgets.slint";

    export struct PlaylistRow {
        title: string,
        count: int,
    }

    export struct TrackRow {
        title: string,
        artist: string,
        time: string,
        playing: bool,
        initial: string,
        hue: float,
        art: image,
        has-art: bool,
        liked: bool,
    }

    global Yt {
        out property <color> bg: #030303;
        out property <color> bar: #212121;
        out property <color> raised: #ffffff1a;
        out property <color> hover: #ffffff14;
        out property <color> divider: #ffffff1a;
        out property <color> text: #ffffff;
        out property <color> secondary: #aaaaaa;
        out property <color> red: #ff0000;
        out property <length> radius: 8px;
    }

    // Vector icons (24x24 viewbox, Material-style), so no font needs media glyphs.
    component Icon inherits Path {
        in property <string> name;
        in property <color> color: Yt.text;
        viewbox-width: 24;
        viewbox-height: 24;
        fill: root.color;
        commands: name == "play" ? "M8 5v14l11-7z"
            : name == "pause" ? "M6 19h4V5H6v14zm8-14v14h4V5h-4z"
            : name == "prev" ? "M6 6h2v12H6zm3.5 6l8.5 6V6z"
            : name == "next" ? "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"
            : name == "heart" ? "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"
            : name == "heart-outline" ? "M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z"
            : name == "shuffle" ? "M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"
            : name == "repeat" ? "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"
            : name == "repeat-one" ? "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2V9h-1l-2 1v1h1.5v4H13z"
            : name == "queue-next" ? "M3 10h11v2H3zm0-4h11v2H3zm0 8h7v2H3zm13-1v-3h-2v3h-3v2h3v3h2v-3h3v-2z"
            : name == "playlist-add" ? "M14 10H2v2h12v-2zm0-4H2v2h12V6zm4 8v-4h-2v4h-4v2h4v4h2v-4h4v-2h-4zM2 16h8v-2H2v2z"
            : name == "volume" ? "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"
            : name == "sync" ? "M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z"
            : name == "search" ? "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"
            : name == "expand" ? "M7.41 15.41L12 10.83l4.59 4.58L18 14l-6-6-6 6z"
            : name == "collapse" ? "M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6z"
            : name == "dropdown" ? "M7 10l5 5 5-5z"
            : name == "busy" ? "M6 10.5a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zm6 0a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zm6 0a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z"
            : "";
    }

    // Round icon button, highlighted on hover like YT Music's player bar.
    component IconButton inherits Rectangle {
        in property <string> icon;
        in property <length> size: 40px;
        in property <length> icon-size: 24px;
        in property <bool> active;
        in property <color> tint: root.active ? Yt.text : Yt.secondary;
        out property <bool> hovered: touch.has-hover;
        callback clicked();
        width: root.size;
        height: root.size;
        border-radius: self.width / 2;
        background: touch.pressed ? #ffffff33 : touch.has-hover ? Yt.raised : transparent;
        touch := TouchArea { clicked => { root.clicked(); } }
        Icon {
            name: root.icon;
            color: touch.has-hover ? Yt.text : root.tint;
            width: root.icon-size;
            height: root.icon-size;
            x: (parent.width - self.width) / 2;
            y: (parent.height - self.height) / 2;
        }
    }

    // Pill button ("Play", "Shuffle"); icon-only when `compact`.
    component Pill inherits Rectangle {
        in property <string> text;
        in property <string> icon;
        in property <bool> filled;
        in property <bool> compact;
        callback clicked();
        height: 36px;
        width: root.compact ? 36px : label.preferred-width + 56px;
        border-radius: self.height / 2;
        background: root.filled ? (touch.has-hover ? #d9d9d9 : #ffffff) : (touch.has-hover ? Yt.raised : transparent);
        border-width: root.filled ? 0 : 1px;
        border-color: #ffffff33;
        touch := TouchArea { clicked => { root.clicked(); } }
        HorizontalLayout {
            padding-left: root.compact ? 8px : 16px;
            padding-right: root.compact ? 8px : 20px;
            spacing: 6px;
            alignment: center;
            Icon {
                name: root.icon;
                color: root.filled ? #030303 : Yt.text;
                width: 20px;
                height: 20px;
                y: (parent.height - self.height) / 2;
            }
            label := Text {
                visible: !root.compact;
                text: root.compact ? "" : root.text;
                color: root.filled ? #030303 : Yt.text;
                font-weight: 500;
                vertical-alignment: center;
            }
        }
    }

    // Thin full-width progress line along the top of the player bar.
    component Progress inherits Rectangle {
        in property <float> ratio;
        callback seek(float);
        height: 12px;
        Rectangle {
            y: (parent.height - self.height) / 2;
            height: touch.has-hover ? 4px : 2px;
            background: #ffffff33;
            Rectangle {
                x: 0;
                width: parent.width * clamp(root.ratio, 0, 1);
                background: Yt.red;
            }
        }
        if touch.has-hover: Rectangle {
            width: 12px;
            height: 12px;
            border-radius: 6px;
            background: Yt.red;
            x: parent.width * clamp(root.ratio, 0, 1) - self.width / 2;
            y: (parent.height - self.height) / 2;
        }
        touch := TouchArea {
            clicked => { root.seek(self.mouse-x / root.width); }
        }
    }

    // Minimal horizontal slider (volume).
    component ThinSlider inherits Rectangle {
        in-out property <float> value;
        callback changed(float);
        width: 90px;
        height: 24px;
        Rectangle {
            y: (parent.height - self.height) / 2;
            height: 3px;
            border-radius: 1.5px;
            background: #ffffff33;
            Rectangle {
                x: 0;
                width: parent.width * clamp(root.value, 0, 1);
                border-radius: 1.5px;
                background: Yt.text;
            }
        }
        Rectangle {
            width: 12px;
            height: 12px;
            border-radius: 6px;
            background: Yt.text;
            x: (parent.width - self.width) * clamp(root.value, 0, 1);
            y: (parent.height - self.height) / 2;
        }
        TouchArea {
            pointer-event(e) => {
                if (e.kind == PointerEventKind.down) { root.set(self.mouse-x); }
            }
            moved => { if (self.pressed) { root.set(self.mouse-x); } }
        }
        function set(x: length) {
            root.value = clamp(x / root.width, 0, 1);
            root.changed(root.value);
        }
    }

    // Album art; until it loads, a tinted tile with the title's initial.
    component Cover inherits Rectangle {
        in property <image> art;
        in property <bool> has-art;
        in property <string> initial;
        in property <float> hue;
        in property <bool> show-play;
        in property <length> size: 48px;
        width: root.size;
        height: root.size;
        border-radius: 4px;
        background: root.has-art ? transparent : hsv(root.hue * 360, 0.45, 0.35);
        if !root.has-art: Text {
            text: root.initial;
            color: #ffffffcc;
            font-size: root.size * 0.42;
            font-weight: 700;
            horizontal-alignment: center;
            vertical-alignment: center;
            width: parent.width;
            height: parent.height;
        }
        if root.has-art: Image {
            source: root.art;
            width: parent.width;
            height: parent.height;
            image-fit: cover;
        }
        if root.show-play: Rectangle {
            background: #00000099;
            Icon {
                name: "play";
                width: root.size * 0.55;
                height: root.size * 0.55;
                x: (parent.width - self.width) / 2;
                y: (parent.height - self.height) / 2;
            }
        }
    }

    // Two-line row: art, title over artist, duration (as in YT Music lists).
    // On hover: save to playlist, play next, like (a liked heart stays).
    component TrackItem inherits Rectangle {
        in property <TrackRow> track;
        in property <bool> selected;
        in property <bool> actions: true;
        callback clicked();
        callback double-clicked();
        callback like();
        callback play-next();
        callback add-to();
        property <bool> hovered: touch.has-hover || like-btn.hovered || next-btn.hovered || add-btn.hovered;
        height: 64px;
        border-radius: 4px;
        background: root.selected ? Yt.raised : touch.has-hover ? Yt.hover : transparent;
        touch := TouchArea {
            clicked => { root.clicked(); }
            double-clicked => { root.double-clicked(); }
        }
        HorizontalLayout {
            padding-left: 8px;
            padding-right: 12px;
            spacing: 14px;
            Cover {
                art: root.track.art;
                has-art: root.track.has-art;
                initial: root.track.initial;
                hue: root.track.hue;
                show-play: root.hovered || root.track.playing;
                y: (parent.height - self.height) / 2;
            }
            VerticalLayout {
                alignment: center;
                spacing: 2px;
                horizontal-stretch: 1;
                Text { text: root.track.title; overflow: elide; color: root.track.playing ? Yt.red : Yt.text; font-weight: 500; font-size: 15px; }
                Text { text: root.track.artist; overflow: elide; color: Yt.secondary; }
            }
            add-btn := IconButton {
                visible: root.actions && root.hovered;
                icon: "playlist-add";
                size: 36px;
                icon-size: 22px;
                y: (parent.height - self.height) / 2;
                clicked => { root.add-to(); }
            }
            next-btn := IconButton {
                visible: root.actions && root.hovered;
                icon: "queue-next";
                size: 36px;
                icon-size: 22px;
                y: (parent.height - self.height) / 2;
                clicked => { root.play-next(); }
            }
            like-btn := IconButton {
                visible: root.actions && (root.hovered || root.track.liked);
                icon: root.track.liked ? "heart" : "heart-outline";
                active: root.track.liked;
                size: 36px;
                icon-size: 22px;
                y: (parent.height - self.height) / 2;
                clicked => { root.like(); }
            }
            Text { text: root.track.time; horizontal-alignment: right; vertical-alignment: center; color: Yt.secondary; min-width: 40px; }
        }
    }

    // Big square cover for the now-playing view, centred in its area.
    component BigCover inherits Rectangle {
        in property <image> art;
        in property <bool> has-art;
        in property <string> initial;
        in property <float> hue;
        property <length> side: min(self.width, self.height);
        Rectangle {
            width: root.side;
            height: root.side;
            x: (parent.width - self.width) / 2;
            y: (parent.height - self.height) / 2;
            border-radius: 8px;
            background: root.has-art ? transparent : hsv(root.hue * 360, 0.45, 0.35);
            drop-shadow-blur: 24px;
            drop-shadow-color: #00000080;
            if root.has-art: Image {
                source: root.art;
                width: parent.width;
                height: parent.height;
                image-fit: cover;
            }
            if !root.has-art: Text {
                text: root.initial;
                color: #ffffffcc;
                font-size: parent.height * 0.4;
                font-weight: 700;
                horizontal-alignment: center;
                vertical-alignment: center;
                width: parent.width;
                height: parent.height;
            }
        }
    }

    // "Up next" panel of the now-playing view.
    component QueuePanel inherits VerticalLayout {
        in property <[TrackRow]> rows;
        in property <string> source;
        callback jump(int);
        spacing: 8px;
        // Tab strip (a single tab: the API offers no lyrics).
        VerticalLayout {
            Text { text: "UP NEXT"; color: Yt.text; font-weight: 600; letter-spacing: 1px; horizontal-alignment: center; height: 36px; vertical-alignment: center; }
            Rectangle { height: 2px; background: Yt.text; }
        }
        if root.source != "": VerticalLayout {
            padding-top: 8px;
            Text { text: "Playing from"; color: Yt.secondary; font-size: 12px; }
            Text { text: root.source; color: Yt.text; font-weight: 600; font-size: 16px; overflow: elide; }
        }
        ListView {
            for t[i] in root.rows: TrackItem {
                track: t;
                actions: false;
                selected: t.playing;
                clicked => { root.jump(i); }
                double-clicked => { root.jump(i); }
            }
        }
    }

    // Playlist entry: title over song count.
    component PlaylistItem inherits Rectangle {
        in property <PlaylistRow> playlist;
        in property <bool> selected;
        callback clicked();
        height: 52px;
        border-radius: Yt.radius;
        background: root.selected ? Yt.raised : touch.has-hover ? Yt.hover : transparent;
        touch := TouchArea { clicked => { root.clicked(); } }
        VerticalLayout {
            padding-left: 12px;
            padding-right: 12px;
            alignment: center;
            Text { text: root.playlist.title; color: Yt.text; overflow: elide; font-weight: root.selected ? 600 : 400; }
            Text { text: root.playlist.count + " songs"; color: Yt.secondary; font-size: 12px; }
        }
    }

    export component MainWindow inherits Window {
        title: "ytm-player";
        // The window's pixel buffers dominate GUI memory; keep the default modest.
        preferred-width: 1040px;
        preferred-height: 660px;
        min-width: 420px;
        min-height: 380px;
        background: Yt.bg;
        default-font-size: 14px;

        in property <[PlaylistRow]> playlists;
        in-out property <int> selected-playlist: -1;
        in property <[TrackRow]> tracks;
        in-out property <int> selected-track: -1;
        in property <string> tracks-title;
        in property <int> track-count;

        in property <string> now-title: "";
        in property <string> now-artist;
        in property <string> now-initial;
        in property <float> now-hue;
        in property <image> now-thumb;
        in property <bool> now-has-thumb;
        in property <image> now-art;
        in property <bool> now-has-art;
        in property <color> now-tint: #303030;
        in property <bool> playing;
        in property <bool> loading;
        in property <float> progress;
        in property <string> position-text;
        in property <float> volume: 0.8;
        in property <bool> shuffle;
        // 0 off, 1 all, 2 one
        in property <int> repeat-mode;
        in property <bool> liked;
        in property <string> status-text;
        in property <bool> status-error;
        in property <bool> syncing;
        in property <bool> searching;
        // Last online search ("" = none); `showing-results` while it's listed.
        in property <string> search-query;
        in property <bool> showing-results;
        // Bumped to empty the search field.
        in property <int> clear-search;
        // Sidebar width set by dragging its edge (0 = automatic).
        in-out property <length> sidebar-width: 0px;
        in property <string> memory-text;

        in-out property <bool> expanded;
        in property <[TrackRow]> queue-rows;
        in property <string> queue-source;

        // Window size, copied in by change handlers rather than bound: layouts
        // depend on the breakpoints, and the window's size constraints depend
        // on the layouts, so a binding would be a loop.
        property <length> win-width: 1040px;
        property <length> win-height: 660px;
        init => { root.win-width = self.width; root.win-height = self.height; }
        changed width => { root.win-width = self.width; }
        changed height => { root.win-height = self.height; }

        // Layout breakpoints by window width.
        property <bool> compact: root.win-width < 1050px;
        property <bool> narrow: root.win-width < 820px;
        property <bool> tiny: root.win-width < 620px;
        property <bool> wide-now: root.win-width >= 880px;
        // Bumped to move keyboard focus into the search field.
        property <int> focus-search;

        callback select-playlist(int);
        callback select-track(int);
        callback play-track(int);
        callback play-all();
        callback shuffle-play();
        callback filter-changed(string);
        callback toggle-pause();
        callback next();
        callback prev();
        callback seek(float);
        callback volume-changed(float);
        callback toggle-shuffle();
        callback cycle-repeat();
        callback toggle-like();
        callback play-next();
        callback add-to(int);
        callback sync();
        callback jump(int);
        callback search-online(string);
        callback show-results();
        callback like-row(int);
        callback sidebar-resized(length);

        forward-focus: keys;
        keys := FocusScope {
            key-pressed(event) => {
                if (event.text == Key.Escape && root.expanded) { root.expanded = false; return accept; }
                if (event.text == " ") { root.toggle-pause(); return accept; }
                if (event.text == "n") { root.next(); return accept; }
                if (event.text == "p") { root.prev(); return accept; }
                if (event.text == "f") { root.toggle-like(); return accept; }
                if (event.text == "/" && !root.expanded) { root.focus-search += 1; return accept; }
                reject
            }

            VerticalLayout {
                // --- main area: library or now playing ---
                Rectangle {
                    vertical-stretch: 1;

                    if !root.expanded: HorizontalLayout {
                        // Sidebar
                        if !root.narrow: Rectangle {
                            width: root.sidebar-width > 0 ? clamp(root.sidebar-width, 180px, max(180px, root.win-width * 0.5)) : (root.compact ? 210px : 260px);
                            Rectangle { x: parent.width - 1px; width: 1px; background: resize.has-hover || resize.pressed ? #ffffff55 : Yt.divider; }
                            VerticalLayout {
                                padding: 12px;
                                padding-top: 20px;
                                spacing: 4px;
                                if root.search-query != "": Rectangle {
                                    height: 52px;
                                    border-radius: Yt.radius;
                                    background: root.showing-results ? Yt.raised : results-touch.has-hover ? Yt.hover : transparent;
                                    results-touch := TouchArea { clicked => { root.show-results(); keys.focus(); } }
                                    HorizontalLayout {
                                        padding-left: 12px;
                                        padding-right: 12px;
                                        spacing: 10px;
                                        Icon { name: "search"; color: Yt.secondary; width: 20px; height: 20px; y: (parent.height - self.height) / 2; }
                                        VerticalLayout {
                                            alignment: center;
                                            Text { text: "Search results"; color: Yt.text; font-weight: root.showing-results ? 600 : 400; }
                                            Text { text: root.search-query; color: Yt.secondary; font-size: 12px; overflow: elide; }
                                        }
                                    }
                                }
                                Text { text: "Library"; color: Yt.secondary; font-size: 12px; font-weight: 600; height: 24px; vertical-alignment: center; x: 12px; }
                                ListView {
                                    for p[i] in root.playlists: PlaylistItem {
                                        playlist: p;
                                        selected: i == root.selected-playlist;
                                        clicked => { root.select-playlist(i); keys.focus(); }
                                    }
                                }
                                Rectangle {
                                    height: 40px;
                                    border-radius: Yt.radius;
                                    background: sync-touch.has-hover ? Yt.hover : transparent;
                                    HorizontalLayout {
                                        padding-left: 12px;
                                        spacing: 10px;
                                        Icon { name: "sync"; color: Yt.secondary; width: 20px; height: 20px; y: (parent.height - self.height) / 2; }
                                        Text { text: root.syncing ? "Syncing…" : "Sync library"; color: Yt.secondary; vertical-alignment: center; }
                                    }
                                    sync-touch := TouchArea { enabled: !root.syncing; clicked => { root.sync(); } }
                                }
                                Text {
                                    text: root.status-text;
                                    color: root.status-error ? #ff6b6b : Yt.secondary;
                                    font-size: 12px;
                                    wrap: word-wrap;
                                    x: 12px;
                                    width: parent.width - 24px;
                                }
                                HorizontalLayout {
                                    padding-left: 12px;
                                    spacing: 8px;
                                    Text { text: root.memory-text; color: #717171; font-size: 11px; }
                                    Text {
                                        text: "Made with Slint";
                                        color: #717171;
                                        font-size: 11px;
                                        TouchArea { clicked => { about-popup.show(); } }
                                    }
                                }
                            }
                            // Drag the right edge to resize; double-click resets.
                            resize := TouchArea {
                                x: parent.width - 4px;
                                width: 8px;
                                mouse-cursor: ew-resize;
                                moved => {
                                    if (self.pressed) {
                                        root.sidebar-width = clamp(parent.width + self.mouse-x - self.pressed-x, 180px, max(180px, root.win-width * 0.5));
                                    }
                                }
                                pointer-event(e) => {
                                    if (e.kind == PointerEventKind.up) { root.sidebar-resized(root.sidebar-width); }
                                }
                                double-clicked => {
                                    root.sidebar-width = 0px;
                                    root.sidebar-resized(0px);
                                }
                            }
                        }

                        // Content
                        VerticalLayout {
                            padding-left: root.tiny ? 12px : 28px;
                            padding-right: root.tiny ? 12px : 24px;
                            padding-top: 16px;
                            spacing: 14px;
                            // Search pill (in its own row: a max-width directly in the
                            // column would cap the whole column's width).
                            HorizontalLayout {
                            Rectangle {
                                height: 40px;
                                max-width: 520px;
                                horizontal-stretch: 1;
                                border-radius: Yt.radius;
                                background: Yt.raised;
                                HorizontalLayout {
                                    padding-left: 12px;
                                    spacing: 4px;
                                    Icon { name: "search"; color: Yt.secondary; width: 22px; height: 22px; y: (parent.height - self.height) / 2; }
                                    Rectangle {
                                        horizontal-stretch: 1;
                                        if search.text == "": Text {
                                            x: 8px;
                                            text: root.tiny ? "Search" : "Search YouTube Music, or type to filter";
                                            color: Yt.secondary;
                                            vertical-alignment: center;
                                            height: parent.height;
                                        }
                                        search := TextInput {
                                            property <int> focus-request: root.focus-search;
                                            changed focus-request => { self.focus(); }
                                            property <int> clear-request: root.clear-search;
                                            changed clear-request => { self.text = ""; }
                                            x: 8px;
                                            width: parent.width - 16px;
                                            color: Yt.text;
                                            vertical-alignment: center;
                                            single-line: true;
                                            edited => { root.filter-changed(self.text); }
                                            accepted => { root.search-online(self.text); keys.focus(); }
                                        }
                                    }
                                    if (search.text != "" && !root.tiny) || root.searching: Text {
                                        text: root.searching ? "Searching…" : "Enter ↵ search";
                                        color: Yt.secondary;
                                        font-size: 12px;
                                        vertical-alignment: center;
                                    }
                                    Rectangle { width: 8px; }
                                }
                            }
                            Rectangle { horizontal-stretch: 0.0001; }
                            }
                            // Playlist header
                            HorizontalLayout {
                                spacing: 12px;
                                VerticalLayout {
                                    alignment: center;
                                    spacing: 2px;
                                    horizontal-stretch: 1;
                                    Text {
                                        text: root.tracks-title;
                                        color: Yt.text;
                                        font-size: root.tiny ? 22px : 30px;
                                        font-weight: 700;
                                        overflow: elide;
                                    }
                                    Text { text: root.track-count + (root.showing-results ? " results" : " songs"); color: Yt.secondary; }
                                }
                                Pill { text: "Play"; icon: "play"; filled: true; compact: root.tiny; y: (parent.height - self.height) / 2; clicked => { root.play-all(); keys.focus(); } }
                                Pill { text: "Shuffle"; icon: "shuffle"; compact: root.tiny; y: (parent.height - self.height) / 2; clicked => { root.shuffle-play(); keys.focus(); } }
                            }
                            // Playlist picker when there is no sidebar.
                            if root.narrow: Rectangle {
                                height: 34px;
                                border-radius: 17px;
                                background: picker-touch.has-hover ? Yt.raised : Yt.hover;
                                HorizontalLayout {
                                    padding-left: 14px;
                                    padding-right: 6px;
                                    spacing: 4px;
                                    Text { text: "Playlists"; color: Yt.text; vertical-alignment: center; horizontal-stretch: 1; }
                                    Icon { name: "dropdown"; width: 24px; height: 24px; y: (parent.height - self.height) / 2; }
                                }
                                picker-touch := TouchArea { clicked => { playlists-popup.show(); } }
                            }
                            Rectangle { height: 1px; background: Yt.divider; }
                            if root.tracks.length == 0: Text {
                                text: search.text != "" ? "No songs match \"" + search.text + "\" — Enter searches YouTube Music"
                                    : root.showing-results ? "Nothing found" : "This playlist is empty";
                                color: Yt.secondary;
                                horizontal-alignment: center;
                                vertical-alignment: center;
                                vertical-stretch: 1;
                            }
                            ListView {
                                visible: root.tracks.length > 0;
                                for t[i] in root.tracks: TrackItem {
                                    track: t;
                                    selected: i == root.selected-track;
                                    clicked => { root.select-track(i); keys.focus(); }
                                    double-clicked => { root.play-track(i); }
                                    like => { root.like-row(i); keys.focus(); }
                                    play-next => { root.selected-track = i; root.play-next(); keys.focus(); }
                                    add-to => { root.selected-track = i; add-popup.show(); }
                                }
                            }
                        }
                    }

                    // --- full-screen now playing ---
                    if root.expanded: Rectangle {
                        background: @linear-gradient(180deg, root.now-tint.mix(Yt.bg, 0.55) 0%, Yt.bg 85%);
                        if root.wide-now: HorizontalLayout {
                            padding: 40px;
                            padding-bottom: 24px;
                            spacing: 48px;
                            BigCover {
                                horizontal-stretch: 3;
                                art: root.now-art;
                                has-art: root.now-has-art;
                                initial: root.now-initial;
                                hue: root.now-hue;
                            }
                            QueuePanel {
                                horizontal-stretch: 2;
                                min-width: 320px;
                                max-width: 560px;
                                rows: root.queue-rows;
                                source: root.queue-source;
                                jump(i) => { root.jump(i); }
                            }
                        }
                        if !root.wide-now: VerticalLayout {
                            padding: 16px;
                            spacing: 16px;
                            BigCover {
                                height: min(root.win-width - 32px, root.win-height * 0.42);
                                art: root.now-art;
                                has-art: root.now-has-art;
                                initial: root.now-initial;
                                hue: root.now-hue;
                            }
                            QueuePanel {
                                vertical-stretch: 1;
                                rows: root.queue-rows;
                                source: root.queue-source;
                                jump(i) => { root.jump(i); }
                            }
                        }
                    }
                }

                // --- player bar ---
                Rectangle {
                    height: 76px;
                    background: Yt.bar;
                    Progress {
                        y: -6px;
                        width: parent.width;
                        ratio: root.progress;
                        seek(r) => { root.seek(r); }
                    }
                    HorizontalLayout {
                        padding-left: 8px;
                        padding-right: 12px;
                        spacing: 4px;
                        // Left: transport + time
                        HorizontalLayout {
                            alignment: start;
                            IconButton { icon: "prev"; active: true; y: (parent.height - self.height) / 2; clicked => { root.prev(); keys.focus(); } }
                            IconButton {
                                icon: root.loading ? "busy" : root.playing ? "pause" : "play";
                                active: true;
                                size: 48px;
                                icon-size: 36px;
                                y: (parent.height - self.height) / 2;
                                clicked => { root.toggle-pause(); keys.focus(); }
                            }
                            IconButton { icon: "next"; active: true; y: (parent.height - self.height) / 2; clicked => { root.next(); keys.focus(); } }
                            if !root.tiny: Text { text: root.position-text; color: Yt.secondary; font-size: 12px; vertical-alignment: center; width: 100px; }
                        }
                        // Centre: current track (click opens the full-screen view)
                        Rectangle {
                            horizontal-stretch: 1;
                            TouchArea {
                                enabled: root.now-title != "";
                                clicked => { root.expanded = !root.expanded; keys.focus(); }
                            }
                            HorizontalLayout {
                                spacing: 12px;
                                alignment: root.compact ? start : center;
                                padding-left: 8px;
                                if root.now-title != "": Cover {
                                    size: 44px;
                                    art: root.now-thumb;
                                    has-art: root.now-has-thumb;
                                    initial: root.now-initial;
                                    hue: root.now-hue;
                                    y: (parent.height - self.height) / 2;
                                }
                                VerticalLayout {
                                    alignment: center;
                                    max-width: 360px;
                                    Text { text: root.now-title != "" ? root.now-title : "Nothing playing"; color: Yt.text; font-weight: 500; overflow: elide; }
                                    Text { text: root.now-artist; color: Yt.secondary; overflow: elide; }
                                }
                                if root.now-title != "" && !root.tiny: IconButton {
                                    icon: root.liked ? "heart" : "heart-outline";
                                    active: root.liked;
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.toggle-like(); keys.focus(); }
                                }
                            }
                        }
                        // Right: volume, modes, queue actions, expand
                        HorizontalLayout {
                            alignment: end;
                            if !root.compact: Icon { name: "volume"; color: Yt.secondary; width: 22px; height: 22px; y: (parent.height - self.height) / 2; }
                            if !root.compact: ThinSlider {
                                value: root.volume;
                                y: (parent.height - self.height) / 2;
                                changed(v) => { root.volume-changed(v); }
                            }
                            if !root.compact: Rectangle { width: 8px; }
                            if !root.tiny: IconButton {
                                icon: root.repeat-mode == 2 ? "repeat-one" : "repeat";
                                active: root.repeat-mode != 0;
                                y: (parent.height - self.height) / 2;
                                clicked => { root.cycle-repeat(); keys.focus(); }
                            }
                            if !root.tiny: IconButton { icon: "shuffle"; active: root.shuffle; y: (parent.height - self.height) / 2; clicked => { root.toggle-shuffle(); keys.focus(); } }
                            if !root.compact: IconButton { icon: "queue-next"; y: (parent.height - self.height) / 2; clicked => { root.play-next(); keys.focus(); } }
                            if !root.compact: IconButton { icon: "playlist-add"; y: (parent.height - self.height) / 2; clicked => { add-popup.show(); } }
                            IconButton {
                                icon: root.expanded ? "collapse" : "expand";
                                active: true;
                                y: (parent.height - self.height) / 2;
                                clicked => { root.expanded = !root.expanded; keys.focus(); }
                            }
                        }
                    }
                }
            }
        }

        add-popup := PopupWindow {
            x: root.win-width - min(300px, root.win-width - 20px);
            y: root.win-height - 76px - self.height;
            width: min(280px, root.win-width - 20px);
            height: min(root.win-height - 140px, 52px + 40px * max(root.playlists.length - 1, 1));
            Rectangle {
                background: #282828;
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    padding: 8px;
                    Text { text: "Save to playlist"; color: Yt.text; font-weight: 600; height: 36px; vertical-alignment: center; x: 8px; }
                    ListView {
                        for p[i] in root.playlists: Rectangle {
                            // Index 0 is Liked music: the heart covers that.
                            height: i > 0 ? 40px : 0px;
                            visible: i > 0;
                            border-radius: 4px;
                            background: pick.has-hover ? Yt.raised : transparent;
                            Text { x: 8px; text: p.title; color: Yt.text; vertical-alignment: center; height: parent.height; }
                            pick := TouchArea { clicked => { root.add-to(i); add-popup.close(); } }
                        }
                    }
                }
            }
        }

        // Playlist picker for narrow windows (no sidebar).
        playlists-popup := PopupWindow {
            x: 12px;
            y: 150px;
            width: min(320px, root.win-width - 24px);
            height: min(root.win-height - 240px, 16px + 52px * (root.playlists.length + (root.search-query != "" ? 1 : 0)));
            Rectangle {
                background: #282828;
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    if root.search-query != "": PlaylistItem {
                        playlist: { title: "Search: " + root.search-query, count: 0 };
                        selected: root.showing-results;
                        clicked => { root.show-results(); playlists-popup.close(); keys.focus(); }
                    }
                    ListView {
                        for p[i] in root.playlists: PlaylistItem {
                            playlist: p;
                            selected: i == root.selected-playlist;
                            clicked => { root.select-playlist(i); playlists-popup.close(); keys.focus(); }
                        }
                    }
                }
            }
        }

        about-popup := PopupWindow {
            x: (root.win-width - min(360px, root.win-width - 20px)) / 2;
            y: (root.win-height - 260px) / 2;
            width: min(360px, root.win-width - 20px);
            height: 260px;
            Rectangle {
                background: #282828;
                border-radius: Yt.radius;
                VerticalLayout {
                    padding: 16px;
                    Text { text: "ytm-player"; font-size: 18px; font-weight: 700; color: Yt.text; }
                    AboutSlint {}
                }
            }
        }
    }
}
