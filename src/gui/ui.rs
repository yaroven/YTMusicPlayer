//! Slint UI, styled after YouTube Music: always dark, red accent, two-line
//! track rows with album art, album / artist / playlist pages with card
//! shelves, a player bar with a full-width progress line and a full-screen
//! "now playing" view (Up next / Lyrics / Related). Responsive by width:
//! - < 1050 px: narrower sidebar, fewer player-bar buttons;
//! - < 820 px: no sidebar (a picker under the title);
//! - < 620 px: icon-only buttons, no time / modes in the bar.
//!
//! The sidebar is resized by dragging its right edge (double-click resets).
//! The search field filters the list as you type; Enter searches YouTube
//! Music. Declarative only; behaviour lives in `gui::run`.

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
        downloaded: bool,
    }

    // An album, artist, playlist or song card.
    export struct CardRow {
        title: string,
        subtitle: string,
        initial: string,
        hue: float,
        art: image,
        has-art: bool,
        // Artists: round art.
        round: bool,
        saved: bool,
    }

    export struct ShelfRow {
        title: string,
        cards: [CardRow],
    }

    // state: 0 upcoming, 1 being sung, 2 sung, 3 not synced.
    export struct LyricLine {
        text: string,
        state: int,
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
            : name == "dislike" ? "M15 3H6c-.83 0-1.54.5-1.84 1.22l-3.02 7.05c-.09.23-.14.47-.14.73v2c0 1.1.9 2 2 2h6.31l-.95 4.57-.03.32c0 .41.17.79.44 1.06L9.83 23l6.59-6.59c.36-.36.58-.86.58-1.41V5c0-1.1-.9-2-2-2zm4 0v12h4V3h-4z"
            : name == "shuffle" ? "M10.59 9.17L5.41 4 4 5.41l5.17 5.17 1.42-1.41zM14.5 4l2.04 2.04L4 18.59 5.41 20 17.96 7.46 20 9.5V4h-5.5zm.33 9.41l-1.41 1.41 3.13 3.13L14.5 20H20v-5.5l-2.04 2.04-3.13-3.13z"
            : name == "repeat" ? "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4z"
            : name == "repeat-one" ? "M7 7h10v3l4-4-4-4v3H5v6h2V7zm10 10H7v-3l-4 4 4 4v-3h12v-6h-2v4zm-4-2V9h-1l-2 1v1h1.5v4H13z"
            : name == "queue-next" ? "M3 10h11v2H3zm0-4h11v2H3zm0 8h7v2H3zm13-1v-3h-2v3h-3v2h3v3h2v-3h3v-2z"
            : name == "queue-add" ? "M19 9H2v2h17V9zm0-4H2v2h17V5zM2 15h13v-2H2v2zm15-2v6l5-3-5-3z"
            : name == "playlist-add" ? "M14 10H2v2h12v-2zm0-4H2v2h12V6zm4 8v-4h-2v4h-4v2h4v4h2v-4h4v-2h-4zM2 16h8v-2H2v2z"
            : name == "volume" ? "M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"
            : name == "sync" ? "M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z"
            : name == "search" ? "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z"
            : name == "expand" || name == "up" ? "M7.41 15.41L12 10.83l4.59 4.58L18 14l-6-6-6 6z"
            : name == "collapse" || name == "down" ? "M7.41 8.59L12 13.17l4.59-4.58L18 10l-6 6-6-6z"
            : name == "dropdown" ? "M7 10l5 5 5-5z"
            : name == "account" ? "M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4zm0 2c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z"
            : name == "close" ? "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z"
            : name == "open" ? "M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z"
            : name == "home" ? "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"
            : name == "history" ? "M13 3a9 9 0 0 0-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42A8.954 8.954 0 0 0 13 21a9 9 0 0 0 0-18zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z"
            : name == "download" ? "M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z"
            : name == "downloaded" ? "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z"
            : name == "album" ? "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 14.5c-2.49 0-4.5-2.01-4.5-4.5S9.51 7.5 12 7.5s4.5 2.01 4.5 4.5-2.01 4.5-4.5 4.5zm0-5.5c-.55 0-1 .45-1 1s.45 1 1 1 1-.45 1-1-.45-1-1-1z"
            : name == "artist" ? "M12 14c1.66 0 2.99-1.34 2.99-3L15 5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm5.3-3c0 3-2.54 5.1-5.3 5.1S6.7 14 6.7 11H5c0 3.41 2.72 6.23 6 6.72V21h2v-3.28c3.28-.48 6-3.3 6-6.72h-1.7z"
            : name == "more" ? "M12 8c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm0 2c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm0 6c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2z"
            : name == "back" ? "M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"
            : name == "radio" ? "M3.24 6.15C2.51 6.43 2 7.17 2 8v12c0 1.1.89 2 2 2h16c1.11 0 2-.9 2-2V8c0-1.11-.89-2-2-2H8.3l8.26-3.34L15.88 1 3.24 6.15zM7 20c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm13-8h-2v-2h-2v2H4V8h16v4z"
            : name == "cast" ? "M21 3H3c-1.1 0-2 .9-2 2v3h2V5h18v14h-7v2h7c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM1 18v3h3c0-1.66-1.34-3-3-3zm0-4v2c2.76 0 5 2.24 5 5h2c0-3.87-3.13-7-7-7zm0-4v2c4.97 0 9 4.03 9 9h2c0-6.08-4.93-11-11-11z"
            : name == "computer" ? "M20 18c1.1 0 1.99-.9 1.99-2L22 6c0-1.1-.9-2-2-2H4c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2H0v2h24v-2h-4zM4 6h16v10H4V6z"
            : name == "moon" ? "M12.34 2.02C6.59 1.82 2 6.42 2 12c0 5.52 4.48 10 10 10 3.71 0 6.93-2.02 8.66-5.02-7.51-.25-12.09-8.43-8.32-14.96z"
            : name == "settings" ? "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.488.488 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"
            : name == "plus" ? "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"
            : name == "check" ? "M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"
            : name == "delete" ? "M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"
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
        animate background { duration: 140ms; easing: ease-out; }
        touch := TouchArea { clicked => { root.clicked(); } }
        Icon {
            name: root.icon;
            color: touch.has-hover ? Yt.text : root.tint;
            animate color { duration: 140ms; }
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
        animate background { duration: 140ms; easing: ease-out; }
        opacity: touch.pressed ? 0.8 : 1;
        animate opacity { duration: 90ms; }
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
            animate height { duration: 120ms; easing: ease-out; }
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
            opacity: 0;
            init => { self.opacity = 1; }
            animate opacity { duration: 220ms; easing: ease-out; }
        }
        Rectangle {
            opacity: root.show-play ? 1 : 0;
            animate opacity { duration: 140ms; }
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
    // On hover: save to playlist, like, more (⋮); a liked heart stays.
    // A red dot over a button: something waits there (an update).
    component Badge inherits Rectangle {
        width: 10px;
        height: 10px;
        border-radius: 5px;
        background: Yt.red;
        border-width: 2px;
        border-color: Yt.bg;
    }

    component TrackItem inherits Rectangle {
        in property <TrackRow> track;
        in property <bool> selected;
        in property <bool> actions: true;
        callback clicked();
        callback double-clicked();
        callback like();
        callback add-to();
        // (x, y) of the ⋮ button in window coordinates, for the menu.
        callback more(length, length);
        // The artist's name was clicked.
        callback artist();
        out property <bool> hovered: touch.has-hover || like-btn.hovered || add-btn.hovered || more-btn.hovered || artist-touch.has-hover;
        height: 64px;
        border-radius: 4px;
        background: root.selected ? Yt.raised : root.hovered ? Yt.hover : transparent;
        animate background { duration: 140ms; easing: ease-out; }
        touch := TouchArea {
            clicked => { root.clicked(); }
            double-clicked => { root.double-clicked(); }
        }
        HorizontalLayout {
            padding-left: 8px;
            padding-right: 8px;
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
                // Only the name itself is a link, not the rest of the row.
                HorizontalLayout {
                    alignment: start;
                    Text {
                        text: root.track.artist;
                        overflow: elide;
                        color: artist-touch.has-hover ? Yt.text : Yt.secondary;
                        artist-touch := TouchArea {
                            mouse-cursor: pointer;
                            clicked => { root.artist(); }
                        }
                    }
                }
            }
            add-btn := IconButton {
                visible: root.actions;
                opacity: root.hovered ? 1 : 0;
                animate opacity { duration: 140ms; }
                icon: "playlist-add";
                size: 36px;
                icon-size: 22px;
                y: (parent.height - self.height) / 2;
                clicked => { root.add-to(); }
            }
            like-btn := IconButton {
                visible: root.actions;
                opacity: root.hovered || root.track.liked ? 1 : 0;
                animate opacity { duration: 140ms; }
                icon: root.track.liked ? "heart" : "heart-outline";
                active: root.track.liked;
                size: 36px;
                icon-size: 22px;
                y: (parent.height - self.height) / 2;
                clicked => { root.like(); }
            }
            if root.track.downloaded: Icon {
                name: "downloaded";
                color: Yt.secondary;
                width: 16px;
                height: 16px;
                y: (parent.height - self.height) / 2;
            }
            Text { text: root.track.time; horizontal-alignment: right; vertical-alignment: center; color: Yt.secondary; min-width: 40px; }
            more-btn := IconButton {
                visible: root.actions;
                tint: root.hovered ? Yt.text : transparent;
                icon: "more";
                size: 36px;
                icon-size: 22px;
                y: (parent.height - self.height) / 2;
                clicked => { root.more(self.absolute-position.x, self.absolute-position.y + self.height); }
            }
        }
    }

    // A card as a list row (search results, saved albums, artists).
    component CardItem inherits Rectangle {
        in property <CardRow> card;
        callback clicked();
        height: 72px;
        border-radius: 4px;
        background: touch.has-hover ? Yt.hover : transparent;
        animate background { duration: 140ms; easing: ease-out; }
        touch := TouchArea { clicked => { root.clicked(); } }
        HorizontalLayout {
            padding-left: 8px;
            padding-right: 12px;
            spacing: 14px;
            Rectangle {
                width: 56px;
                height: 56px;
                y: (parent.height - self.height) / 2;
                border-radius: root.card.round ? 28px : 4px;
                clip: true;
                background: root.card.has-art ? transparent : hsv(root.card.hue * 360, 0.45, 0.35);
                if root.card.has-art: Image { source: root.card.art; width: parent.width; height: parent.height; image-fit: cover; opacity: 0; init => { self.opacity = 1; } animate opacity { duration: 220ms; } }
                if !root.card.has-art: Text { text: root.card.initial; color: #ffffffcc; font-size: 22px; font-weight: 700; horizontal-alignment: center; vertical-alignment: center; width: parent.width; height: parent.height; }
            }
            VerticalLayout {
                alignment: center;
                spacing: 2px;
                horizontal-stretch: 1;
                Text { text: root.card.title; overflow: elide; color: Yt.text; font-weight: 500; font-size: 15px; }
                Text { text: root.card.subtitle; overflow: elide; color: Yt.secondary; }
            }
            if root.card.saved: Icon { name: "check"; color: Yt.secondary; width: 20px; height: 20px; y: (parent.height - self.height) / 2; }
        }
    }

    // A tile in a horizontal shelf.
    component Card inherits Rectangle {
        in property <CardRow> card;
        in property <length> size: 150px;
        callback clicked();
        width: root.size;
        height: root.size + 52px;
        touch := TouchArea { clicked => { root.clicked(); } }
        Rectangle {
            y: 0;
            width: root.size;
            height: root.size;
            border-radius: root.card.round ? root.size / 2 : 6px;
            clip: true;
            background: root.card.has-art ? transparent : hsv(root.card.hue * 360, 0.45, 0.35);
            if root.card.has-art: Image { source: root.card.art; width: parent.width; height: parent.height; image-fit: cover; opacity: 0; init => { self.opacity = 1; } animate opacity { duration: 220ms; } }
            if !root.card.has-art: Text { text: root.card.initial; color: #ffffffcc; font-size: root.size * 0.35; font-weight: 700; horizontal-alignment: center; vertical-alignment: center; width: parent.width; height: parent.height; }
            Rectangle {
                background: #00000055;
                opacity: touch.has-hover ? 1 : 0;
                animate opacity { duration: 160ms; }
            }
        }
        Text {
            y: root.size + 6px;
            width: root.size;
            text: root.card.title;
            color: Yt.text;
            font-weight: 500;
            overflow: elide;
            horizontal-alignment: root.card.round ? center : left;
        }
        Text {
            y: root.size + 26px;
            width: root.size;
            text: root.card.subtitle;
            color: Yt.secondary;
            font-size: 12px;
            overflow: elide;
            horizontal-alignment: root.card.round ? center : left;
        }
    }

    // A titled row of cards that scrolls sideways.
    component ShelfView inherits VerticalLayout {
        in property <ShelfRow> shelf;
        in property <length> card-size: 150px;
        callback open(int);
        spacing: 10px;
        Text { text: root.shelf.title; color: Yt.text; font-size: 20px; font-weight: 700; }
        Flickable {
            height: root.card-size + 56px;
            content-width: row.preferred-width;
            row := HorizontalLayout {
                spacing: 16px;
                for c[i] in root.shelf.cards: Card {
                    card: c;
                    size: root.card-size;
                    clicked => { root.open(i); }
                }
            }
        }
    }

    // A chip in a row of tabs.
    component Chip inherits Rectangle {
        in property <string> text;
        in property <bool> selected;
        callback clicked();
        height: 32px;
        width: label.preferred-width + 28px;
        border-radius: 8px;
        background: root.selected ? Yt.text : touch.has-hover ? #ffffff33 : Yt.raised;
        animate background { duration: 160ms; easing: ease-out; }
        touch := TouchArea { clicked => { root.clicked(); } }
        label := Text {
            text: root.text;
            color: root.selected ? #030303 : Yt.text;
            animate color { duration: 160ms; }
            font-weight: 500;
            horizontal-alignment: center;
            vertical-alignment: center;
            width: parent.width;
            height: parent.height;
        }
    }

    // On/off switch with a label.
    component Toggle inherits Rectangle {
        in property <string> text;
        in property <bool> on;
        callback toggled();
        height: 36px;
        touch := TouchArea { clicked => { root.toggled(); } }
        HorizontalLayout {
            spacing: 12px;
            Text { text: root.text; color: Yt.text; vertical-alignment: center; horizontal-stretch: 1; }
            Rectangle {
                width: 40px;
                height: 22px;
                y: (parent.height - self.height) / 2;
                border-radius: 11px;
                background: root.on ? rgb(62, 166, 255) : #ffffff33;
                animate background { duration: 160ms; }
                Rectangle {
                    width: 18px;
                    height: 18px;
                    border-radius: 9px;
                    background: Yt.text;
                    x: root.on ? parent.width - self.width - 2px : 2px;
                    y: 2px;
                    animate x { duration: 160ms; easing: ease-out; }
                }
            }
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
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 300ms; easing: ease-out; }
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

    // "Up next" / "Lyrics" / "Related" panel of the now-playing view.
    component QueuePanel inherits VerticalLayout {
        in property <[TrackRow]> rows;
        in property <string> source;
        in property <bool> autoplay;
        in property <[LyricLine]> lyrics;
        in property <int> lyrics-current;
        in property <string> lyrics-source;
        in property <bool> lyrics-loading;
        in property <[ShelfRow]> related;
        in-out property <int> tab;
        callback jump(int);
        callback queue-action(int, string);
        callback clear-queue();
        callback toggle-autoplay();
        callback related-open(int, int);
        spacing: 8px;
        tabs := Rectangle {
            height: 38px;
            HorizontalLayout {
                for title[i] in ["UP NEXT", "LYRICS", "RELATED"]: Text {
                    horizontal-stretch: 1;
                    text: title;
                    color: i == root.tab ? Yt.text : Yt.secondary;
                    animate color { duration: 180ms; }
                    font-weight: 600;
                    letter-spacing: 1px;
                    horizontal-alignment: center;
                    height: 36px;
                    vertical-alignment: center;
                    TouchArea { clicked => { root.tab = i; } }
                }
            }
            Rectangle { y: parent.height - 2px; height: 2px; background: Yt.divider; }
            // One underline that slides to the chosen tab.
            Rectangle {
                y: parent.height - 2px;
                height: 2px;
                width: parent.width / 3;
                x: root.tab * parent.width / 3;
                animate x { duration: 220ms; easing: ease-in-out; }
                background: Yt.text;
            }
        }
        if root.tab == 0: Rectangle {
          opacity: 0;
          init => { self.opacity = 1; }
          animate opacity { duration: 200ms; }
          VerticalLayout {
            spacing: 6px;
            HorizontalLayout {
                padding-top: 6px;
                spacing: 8px;
                VerticalLayout {
                    horizontal-stretch: 1;
                    alignment: center;
                    if root.source != "": Text { text: "Playing from"; color: Yt.secondary; font-size: 12px; }
                    if root.source != "": Text { text: root.source; color: Yt.text; font-weight: 600; font-size: 16px; overflow: elide; }
                }
                Toggle { text: "Autoplay"; on: root.autoplay; width: 140px; toggled => { root.toggle-autoplay(); } }
                IconButton { icon: "delete"; size: 36px; icon-size: 20px; y: (parent.height - self.height) / 2; clicked => { root.clear-queue(); } }
            }
            ListView {
                vertical-stretch: 1;
                for t[i] in root.rows: Rectangle {
                    height: 64px;
                    // Row 0 is the playing track: no editing.
                    property <bool> hot: i > 0 && (item.hovered || up.hovered || down.hovered || remove.hovered);
                    item := TrackItem {
                        track: t;
                        actions: false;
                        selected: t.playing;
                        clicked => { root.jump(i); }
                        double-clicked => { root.jump(i); }
                        artist => { root.queue-action(i, "artist"); }
                    }
                    HorizontalLayout {
                        alignment: end;
                        padding-right: 52px;
                        up := IconButton { visible: hot; icon: "up"; size: 32px; icon-size: 20px; y: (parent.height - self.height) / 2; clicked => { root.queue-action(i, "up"); } }
                        down := IconButton { visible: hot; icon: "down"; size: 32px; icon-size: 20px; y: (parent.height - self.height) / 2; clicked => { root.queue-action(i, "down"); } }
                        remove := IconButton { visible: hot; icon: "close"; size: 32px; icon-size: 20px; y: (parent.height - self.height) / 2; clicked => { root.queue-action(i, "remove"); } }
                    }
                }
            }
          }
        }
        if root.tab == 1: Rectangle {
            vertical-stretch: 1;
            opacity: 0;
            init => { self.opacity = 1; }
            animate opacity { duration: 200ms; }
            if root.lyrics.length == 0: Text {
                text: root.lyrics-loading ? "Looking for lyrics…" : "No lyrics for this song";
                color: Yt.secondary;
                horizontal-alignment: center;
                vertical-alignment: center;
                width: parent.width;
                height: parent.height;
            }
            lyrics-view := Flickable {
                // Keeps the sung line in the upper third.
                property <length> line-height: 34px;
                content-height: lines.preferred-height;
                property <int> current: root.lyrics-current;
                changed current => {
                    if (self.current >= 0) {
                        self.content-y = min(0px, max(self.height - self.content-height, -(self.current * self.line-height) + self.height / 3));
                    }
                }
                animate content-y { duration: 300ms; easing: ease-out; }
                lines := VerticalLayout {
                    padding-top: 8px;
                    padding-bottom: 40px;
                    for l in root.lyrics: Text {
                        min-height: lyrics-view.line-height;
                        text: l.text;
                        wrap: word-wrap;
                        font-size: l.state == 3 ? 16px : 20px;
                        font-weight: l.state == 1 ? 700 : 500;
                        color: l.state == 1 ? Yt.text : l.state == 2 ? #ffffff80 : l.state == 3 ? Yt.text : #ffffffb0;
                        animate color { duration: 300ms; }
                    }
                    if root.lyrics-source != "": Text { text: root.lyrics-source; color: Yt.secondary; font-size: 12px; }
                }
            }
        }
        if root.tab == 2: Flickable {
            vertical-stretch: 1;
            opacity: 0;
            init => { self.opacity = 1; }
            animate opacity { duration: 200ms; }
            content-height: shelves.preferred-height;
            shelves := VerticalLayout {
                spacing: 20px;
                padding-top: 8px;
                if root.related.length == 0: Text { text: "Loading…"; color: Yt.secondary; }
                for s[si] in root.related: ShelfView {
                    shelf: s;
                    card-size: 120px;
                    open(ci) => { root.related-open(si, ci); }
                }
            }
        }
    }

    // Single-line text field (account dialog).
    component Field inherits Rectangle {
        in-out property <string> text;
        in property <string> placeholder;
        in property <bool> password;
        height: 40px;
        border-radius: Yt.radius;
        background: Yt.raised;
        border-width: 1px;
        border-color: input.has-focus ? #ffffff66 : transparent;
        animate border-color { duration: 160ms; }
        if root.text == "": Text {
            x: 12px;
            text: root.placeholder;
            color: Yt.secondary;
            vertical-alignment: center;
            height: parent.height;
        }
        input := TextInput {
            x: 12px;
            width: parent.width - 24px;
            text <=> root.text;
            color: Yt.text;
            vertical-alignment: center;
            single-line: true;
            input-type: root.password ? InputType.password : InputType.text;
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
        animate background { duration: 140ms; easing: ease-out; }
        touch := TouchArea { clicked => { root.clicked(); } }
        VerticalLayout {
            padding-left: 12px;
            padding-right: 12px;
            alignment: center;
            Text { text: root.playlist.title; color: Yt.text; overflow: elide; font-weight: root.selected ? 600 : 400; }
            Text { text: root.playlist.count + " songs"; color: Yt.secondary; font-size: 12px; }
        }
    }

    // A sidebar destination: icon and label.
    component NavItem inherits Rectangle {
        in property <string> icon;
        in property <string> text;
        in property <bool> selected;
        callback clicked();
        height: 40px;
        border-radius: Yt.radius;
        background: root.selected ? Yt.raised : touch.has-hover ? Yt.hover : transparent;
        animate background { duration: 140ms; easing: ease-out; }
        touch := TouchArea { clicked => { root.clicked(); } }
        HorizontalLayout {
            padding-left: 12px;
            spacing: 14px;
            Icon { name: root.icon; color: root.selected ? Yt.text : Yt.secondary; width: 22px; height: 22px; y: (parent.height - self.height) / 2; }
            Text { text: root.text; color: Yt.text; font-weight: root.selected ? 600 : 400; vertical-alignment: center; overflow: elide; }
        }
    }

    // A menu entry.
    component MenuItem inherits Rectangle {
        in property <string> icon;
        in property <string> text;
        callback clicked();
        height: 40px;
        border-radius: 4px;
        background: touch.has-hover ? Yt.raised : transparent;
        animate background { duration: 120ms; }
        touch := TouchArea { clicked => { root.clicked(); } }
        HorizontalLayout {
            padding-left: 12px;
            spacing: 14px;
            Icon { name: root.icon; color: Yt.secondary; width: 20px; height: 20px; y: (parent.height - self.height) / 2; }
            Text { text: root.text; color: Yt.text; vertical-alignment: center; }
        }
    }

    // A modal card over a dimmed window.
    component Dialog inherits Rectangle {
        in property <string> title;
        in property <length> card-width: 420px;
        callback close();
        background: #000000b3;
        // Fades in, the card rises a little.
        opacity: 0;
        property <length> rise: 16px;
        init => { self.opacity = 1; self.rise = 0; }
        animate opacity { duration: 180ms; easing: ease-out; }
        animate rise { duration: 220ms; easing: ease-out; }
        TouchArea { } // swallow clicks behind the card
        Rectangle {
            width: min(root.card-width, root.width - 24px);
            height: min(body.preferred-height, root.height - 24px);
            x: (parent.width - self.width) / 2;
            y: (parent.height - self.height) / 2 + root.rise;
            background: #282828;
            border-radius: 12px;
            drop-shadow-blur: 24px;
            drop-shadow-color: #000000aa;
            clip: true;
            Flickable {
                content-height: body.preferred-height;
                body := VerticalLayout {
                    padding: 20px;
                    spacing: 12px;
                    HorizontalLayout {
                        Text { text: root.title; color: Yt.text; font-size: 20px; font-weight: 700; vertical-alignment: center; horizontal-stretch: 1; }
                        IconButton { icon: "close"; clicked => { root.close(); } }
                    }
                    @children
                }
            }
        }
    }

    export component MainWindow inherits Window {
        title: "ytm-player";
        // Taskbar / window-switcher icon on Windows and X11 (macOS uses the bundle's).
        icon: @image-url("../../assets/ytm-player-64.png");
        // The window's pixel buffers dominate GUI memory; keep the default modest.
        preferred-width: 1040px;
        preferred-height: 660px;
        min-width: 420px;
        min-height: 380px;
        background: Yt.bg;
        default-font-size: 14px;

        // --- library and the shown list ---
        in property <[PlaylistRow]> playlists;
        // "Save to playlist" choices: every playlist except Liked music.
        // add-to(i) saves to choice i; -1 asks for a new playlist with the
        // track, -2 for an empty one.
        in property <[string]> add-choices;
        in-out property <int> selected-playlist: -1;
        // Sidebar destination: 0 none, 1 home, 2 history, 3 downloads,
        // 4 albums, 5 artists, 6 search results.
        in property <int> nav;
        in property <[TrackRow]> tracks;
        in-out property <int> selected-track: -1;
        in property <[CardRow]> cards;
        in property <[ShelfRow]> shelves;
        // 0: track list, 1: cards, 2: page (tracks + shelves).
        in property <int> list-mode;
        in property <string> tracks-title;
        in property <string> tracks-subtitle;
        in property <image> page-art;
        in property <bool> has-page-art;
        in property <bool> page-art-round;
        // 0 none, 1 album, 2 artist, 3 playlist page.
        in property <int> page-kind;
        in property <bool> page-saved;
        // A newer release ("" = none), and whether it's being installed.
        in property <string> update-version;
        in property <bool> updating;
        in property <string> app-version;
        // All songs of the page (the list shows only a few of them).
        in property <int> page-songs;
        // A playlist of the user's (not Liked music): rename / delete.
        in property <bool> own-playlist;
        in property <bool> can-back;
        in property <bool> searching;
        in property <string> search-query;
        // 0 songs, 1 albums, 2 artists, 3 playlists (when showing results).
        in property <int> search-kind;
        // Bumped to empty the search field.
        in property <int> clear-search;

        // --- now playing ---
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
        in property <bool> autoplay;
        // "" or "23 min" / "end of track".
        in property <string> sleep-text;
        // Name of the Cast device playing ("" = this computer).
        in property <string> casting;
        in property <[string]> cast-devices;
        in property <bool> cast-scanning;
        in-out property <bool> expanded;
        in-out property <int> now-tab;
        in property <[TrackRow]> queue-rows;
        in property <string> queue-source;
        in property <[LyricLine]> lyrics;
        in property <int> lyrics-current: -1;
        in property <string> lyrics-source;
        in property <bool> lyrics-loading;
        in property <[ShelfRow]> related;

        // --- status, account, settings ---
        in property <string> status-text;
        in property <bool> status-error;
        in property <bool> syncing;
        in property <string> memory-text;
        in-out property <bool> account-open;
        in property <bool> signed-in;
        in property <bool> signing-in;
        in property <string> client-id;
        // Name of a client_secret_*.json found in Downloads ("" = none).
        in property <string> download-file;
        in-out property <bool> settings-open;
        in property <bool> set-normalize;
        in property <bool> set-notifications;
        in property <bool> set-tray;
        in property <float> set-crossfade;
        // Browser whose YouTube login age-restricted songs use ("" = off).
        in property <string> set-cookies;
        // Sidebar width set by dragging its edge (0 = automatic).
        in-out property <length> sidebar-width: 0px;

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
        // Row the ⋮ menu is for (-1: the playing track) and where it opens.
        property <int> menu-row: -1;
        property <length> menu-x;
        property <length> menu-y;
        // Playlist name dialog: 0 closed, 1 new, 2 rename.
        property <int> name-dialog;
        property <bool> delete-dialog;

        callback select-playlist(int);
        callback select-track(int);
        callback play-track(int);
        callback play-all();
        callback shuffle-play();
        callback start-radio();
        callback filter-changed(string);
        callback toggle-pause();
        callback next();
        callback prev();
        callback seek(float);
        callback volume-changed(float);
        callback toggle-shuffle();
        callback cycle-repeat();
        callback toggle-like();
        callback dislike-now();
        callback play-next();
        callback add-to(int);
        callback sync();
        callback jump(int);
        callback search-online(string);
        callback search-category(int);
        callback like-row(int);
        // row (-1: the playing track), action: "next", "queue", "radio",
        // "download", "dislike", "remove", "add".
        callback row-action(int, string);
        callback open-card(int);
        callback open-shelf-card(int, int);
        callback open-related(int, int);
        callback nav-to(int);
        callback go-back();
        callback toggle-saved();
        callback download-all();
        callback new-playlist(string);
        callback rename-playlist(string);
        callback delete-playlist();
        callback queue-action(int, string);
        callback clear-queue();
        callback toggle-autoplay();
        callback set-sleep(int);
        callback cast-scan();
        // Device index, or -1 for this computer.
        callback cast-to(int);
        callback now-tab-changed(int);
        callback setting-toggled(string);
        callback crossfade-changed(float);
        callback cookies-cycle();
        callback show-all-songs();
        callback update-now();
        callback check-updates();
        callback sidebar-resized(length);
        callback account-opened();
        callback save-client(string, string);
        callback import-downloaded();
        callback sign-in();
        callback sign-out();
        callback open-console();

        // Lyrics / Related are fetched only while their tab is on screen.
        changed now-tab => { root.now-tab-changed(root.expanded ? root.now-tab : 0); }
        changed expanded => { root.now-tab-changed(root.expanded ? root.now-tab : 0); }

        forward-focus: keys;
        keys := FocusScope {
            key-pressed(event) => {
                if (event.text == Key.Escape && root.account-open) { root.account-open = false; return accept; }
                if (event.text == Key.Escape && root.settings-open) { root.settings-open = false; return accept; }
                if (event.text == Key.Escape && root.expanded) { root.expanded = false; return accept; }
                if (event.text == Key.Escape && root.can-back) { root.go-back(); return accept; }
                if (event.text == Key.Backspace && root.can-back) { root.go-back(); return accept; }
                if (event.text == " ") { root.toggle-pause(); return accept; }
                if (event.text == "n") { root.next(); return accept; }
                if (event.text == "p") { root.prev(); return accept; }
                if (event.text == "f") { root.toggle-like(); return accept; }
                if (event.text == "t") { root.expanded = true; root.now-tab = 1; return accept; }
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
                                padding-top: 16px;
                                spacing: 2px;
                                NavItem { icon: "home"; text: "Home"; selected: root.nav == 1; clicked => { root.nav-to(1); keys.focus(); } }
                                if root.search-query != "": NavItem {
                                    icon: "search";
                                    text: "Search: " + root.search-query;
                                    selected: root.nav == 6;
                                    clicked => { root.nav-to(6); keys.focus(); }
                                }
                                Rectangle { height: 8px; }
                                HorizontalLayout {
                                    padding-left: 12px;
                                    Text { text: "Library"; color: Yt.secondary; font-size: 12px; font-weight: 600; vertical-alignment: center; horizontal-stretch: 1; }
                                    IconButton { icon: "plus"; size: 28px; icon-size: 18px; clicked => { root.add-to(-2); root.name-dialog = 1; } }
                                }
                                NavItem { icon: "history"; text: "History"; selected: root.nav == 2; clicked => { root.nav-to(2); keys.focus(); } }
                                NavItem { icon: "download"; text: "Downloads"; selected: root.nav == 3; clicked => { root.nav-to(3); keys.focus(); } }
                                NavItem { icon: "album"; text: "Albums"; selected: root.nav == 4; clicked => { root.nav-to(4); keys.focus(); } }
                                NavItem { icon: "artist"; text: "Artists"; selected: root.nav == 5; clicked => { root.nav-to(5); keys.focus(); } }
                                Rectangle { height: 1px; background: Yt.divider; }
                                ListView {
                                    for p[i] in root.playlists: PlaylistItem {
                                        playlist: p;
                                        selected: i == root.selected-playlist;
                                        clicked => { root.select-playlist(i); keys.focus(); }
                                    }
                                }
                                if root.update-version != "": Pill {
                                    text: root.updating ? "Updating…" : "Update to " + root.update-version;
                                    icon: "download";
                                    filled: true;
                                    clicked => { root.settings-open = true; }
                                }
                                HorizontalLayout {
                                    spacing: 2px;
                                    NavItem {
                                        horizontal-stretch: 1;
                                        icon: "account";
                                        text: root.signed-in ? "Account" : "Sign in";
                                        clicked => { root.account-opened(); root.account-open = true; }
                                    }
                                    IconButton { icon: "sync"; size: 40px; icon-size: 20px; clicked => { root.sync(); } }
                                    Rectangle {
                                        width: 40px;
                                        IconButton { icon: "settings"; size: 40px; icon-size: 20px; clicked => { root.settings-open = true; } }
                                        if root.update-version != "": Badge { x: 26px; y: 6px; }
                                    }
                                }
                                Text {
                                    text: root.syncing ? "Syncing…" : root.status-text;
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
                            spacing: 12px;
                            // Back + search pill (in its own row: a max-width directly
                            // in the column would cap the whole column's width).
                            HorizontalLayout {
                                spacing: 8px;
                                if root.can-back: IconButton { icon: "back"; active: true; y: (parent.height - self.height) / 2; clicked => { root.go-back(); keys.focus(); } }
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
                                if root.narrow: IconButton {
                                    icon: "account";
                                    tint: root.signed-in ? Yt.secondary : Yt.red;
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.account-opened(); root.account-open = true; }
                                }
                            }
                            // Header: art, title, actions
                            HorizontalLayout {
                                spacing: 16px;
                                if root.has-page-art && !root.tiny: Rectangle {
                                    width: root.compact ? 96px : 128px;
                                    height: self.width;
                                    border-radius: root.page-art-round ? self.width / 2 : 6px;
                                    clip: true;
                                    Image { source: root.page-art; width: parent.width; height: parent.height; image-fit: cover; }
                                }
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
                                    Text { text: root.tracks-subtitle; color: Yt.secondary; overflow: elide; }
                                }
                                if root.list-mode != 1 && root.tracks.length > 0: Pill { text: "Play"; icon: "play"; filled: true; compact: root.tiny || (root.compact && root.page-kind != 0); y: (parent.height - self.height) / 2; clicked => { root.play-all(); keys.focus(); } }
                                if root.list-mode != 1 && root.tracks.length > 0 && !root.tiny: Pill { text: "Shuffle"; icon: "shuffle"; compact: root.compact; y: (parent.height - self.height) / 2; clicked => { root.shuffle-play(); keys.focus(); } }
                                if root.list-mode != 1 && root.tracks.length > 0 && root.page-kind != 0 && !root.tiny: Pill { text: "Radio"; icon: "radio"; compact: true; y: (parent.height - self.height) / 2; clicked => { root.start-radio(); keys.focus(); } }
                                if root.page-kind == 1 || root.page-kind == 3: Pill {
                                    text: root.page-saved ? "Saved" : "Save";
                                    icon: root.page-saved ? "check" : "plus";
                                    compact: root.compact;
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.toggle-saved(); }
                                }
                                if root.page-kind == 2: Pill {
                                    text: root.page-saved ? "Following" : "Follow";
                                    icon: root.page-saved ? "check" : "plus";
                                    compact: root.compact;
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.toggle-saved(); }
                                }
                                if root.list-mode != 1 && root.tracks.length > 0 && !root.tiny: IconButton {
                                    icon: "download";
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.download-all(); }
                                }
                                if root.own-playlist: IconButton {
                                    icon: "more";
                                    y: (parent.height - self.height) / 2;
                                    clicked => { playlist-menu.show(); }
                                }
                            }
                            // Search categories
                            if root.nav == 6: HorizontalLayout {
                                spacing: 8px;
                                alignment: start;
                                for label[k] in ["Songs", "Albums", "Artists", "Playlists"]: Chip {
                                    text: label;
                                    selected: k == root.search-kind;
                                    clicked => { root.search-category(k); keys.focus(); }
                                }
                            }
                            // Playlist picker when there is no sidebar.
                            if root.narrow: HorizontalLayout {
                                spacing: 8px;
                                Rectangle {
                                    height: 34px;
                                    horizontal-stretch: 1;
                                    border-radius: 17px;
                                    background: picker-touch.has-hover ? Yt.raised : Yt.hover;
                                    HorizontalLayout {
                                        padding-left: 14px;
                                        padding-right: 6px;
                                        spacing: 4px;
                                        Text { text: "Library"; color: Yt.text; vertical-alignment: center; horizontal-stretch: 1; }
                                        Icon { name: "dropdown"; width: 24px; height: 24px; y: (parent.height - self.height) / 2; }
                                    }
                                    picker-touch := TouchArea { clicked => { playlists-popup.show(); } }
                                }
                                Rectangle {
                                    width: 40px;
                                    IconButton { icon: "settings"; y: (parent.height - self.height) / 2; clicked => { root.settings-open = true; } }
                                    if root.update-version != "": Badge { x: 26px; y: (parent.height - 40px) / 2 + 6px; }
                                }
                            }
                            Rectangle { height: 1px; background: Yt.divider; }
                            if (root.list-mode == 0 && root.tracks.length == 0) || (root.list-mode == 1 && root.cards.length == 0): Text {
                                text: search.text != "" ? "No songs match \"" + search.text + "\" — Enter searches YouTube Music"
                                    : root.nav == 6 ? "Nothing found"
                                    : root.nav == 3 ? "Downloaded songs play offline — download from a song's ⋮ menu"
                                    : root.nav == 2 ? "Songs you play show up here"
                                    : root.nav == 4 ? "Save albums from their page"
                                    : root.nav == 5 ? "Follow artists from their page (sync brings your subscriptions)"
                                    : "This playlist is empty";
                                color: Yt.secondary;
                                horizontal-alignment: center;
                                vertical-alignment: center;
                                wrap: word-wrap;
                                vertical-stretch: 1;
                            }
                            // Track list (virtualized: playlists can be long).
                            if root.list-mode == 0 && root.tracks.length > 0: ListView {
                                vertical-stretch: 1;
                                opacity: 0;
                                init => { self.opacity = 1; }
                                animate opacity { duration: 180ms; easing: ease-out; }
                                for t[i] in root.tracks: TrackItem {
                                    track: t;
                                    selected: i == root.selected-track;
                                    clicked => { root.select-track(i); keys.focus(); }
                                    double-clicked => { root.play-track(i); }
                                    like => { root.like-row(i); keys.focus(); }
                                    add-to => { root.selected-track = i; add-popup.show(); }
                                    more(x, y) => { root.menu-row = i; root.menu-x = x; root.menu-y = y; row-menu.show(); }
                                    artist => { root.row-action(i, "artist"); keys.focus(); }
                                }
                            }
                            // Cards: albums, artists, playlists.
                            if root.list-mode == 1 && root.cards.length > 0: ListView {
                                vertical-stretch: 1;
                                opacity: 0;
                                init => { self.opacity = 1; }
                                animate opacity { duration: 180ms; easing: ease-out; }
                                for c[i] in root.cards: CardItem {
                                    card: c;
                                    clicked => { root.open-card(i); keys.focus(); }
                                }
                            }
                            // Page: a few tracks, then shelves of cards.
                            if root.list-mode == 2: Flickable {
                                vertical-stretch: 1;
                                opacity: 0;
                                init => { self.opacity = 1; }
                                animate opacity { duration: 180ms; easing: ease-out; }
                                content-height: page.preferred-height;
                                page := VerticalLayout {
                                    spacing: 24px;
                                    padding-bottom: 24px;
                                    VerticalLayout {
                                        for t[i] in root.tracks: TrackItem {
                                            track: t;
                                            selected: i == root.selected-track;
                                            clicked => { root.select-track(i); keys.focus(); }
                                            double-clicked => { root.play-track(i); }
                                            like => { root.like-row(i); keys.focus(); }
                                            add-to => { root.selected-track = i; add-popup.show(); }
                                            more(x, y) => { root.menu-row = i; root.menu-x = x; root.menu-y = y; row-menu.show(); }
                                            artist => { root.row-action(i, "artist"); keys.focus(); }
                                        }
                                        // Pages show a few songs; the rest open as a list.
                                        if root.page-songs > root.tracks.length: HorizontalLayout {
                                            alignment: start;
                                            padding-top: 8px;
                                            Pill {
                                                text: "Show all " + root.page-songs + " songs";
                                                clicked => { root.show-all-songs(); keys.focus(); }
                                            }
                                        }
                                    }
                                    for s[si] in root.shelves: ShelfView {
                                        shelf: s;
                                        card-size: root.tiny ? 120px : 150px;
                                        open(ci) => { root.open-shelf-card(si, ci); keys.focus(); }
                                    }
                                }
                            }
                        }
                    }

                    // --- full-screen now playing ---
                    if root.expanded: Rectangle {
                        background: @linear-gradient(180deg, root.now-tint.mix(Yt.bg, 0.55) 0%, Yt.bg 85%);
                        // Slides up from the player bar.
                        property <length> slide: 48px;
                        opacity: 0;
                        y: self.slide;
                        init => { self.opacity = 1; self.slide = 0; }
                        animate opacity { duration: 200ms; easing: ease-out; }
                        animate slide { duration: 260ms; easing: ease-out; }
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
                                autoplay: root.autoplay;
                                lyrics: root.lyrics;
                                lyrics-current: root.lyrics-current;
                                lyrics-source: root.lyrics-source;
                                lyrics-loading: root.lyrics-loading;
                                related: root.related;
                                tab <=> root.now-tab;
                                jump(i) => { root.jump(i); }
                                queue-action(i, a) => { root.queue-action(i, a); }
                                clear-queue => { root.clear-queue(); }
                                toggle-autoplay => { root.toggle-autoplay(); }
                                related-open(s, c) => { root.open-related(s, c); }
                            }
                        }
                        if !root.wide-now: VerticalLayout {
                            padding: 16px;
                            spacing: 16px;
                            BigCover {
                                height: min(root.win-width - 32px, root.win-height * 0.36);
                                art: root.now-art;
                                has-art: root.now-has-art;
                                initial: root.now-initial;
                                hue: root.now-hue;
                            }
                            QueuePanel {
                                vertical-stretch: 1;
                                rows: root.queue-rows;
                                source: root.queue-source;
                                autoplay: root.autoplay;
                                lyrics: root.lyrics;
                                lyrics-current: root.lyrics-current;
                                lyrics-source: root.lyrics-source;
                                lyrics-loading: root.lyrics-loading;
                                related: root.related;
                                tab <=> root.now-tab;
                                jump(i) => { root.jump(i); }
                                queue-action(i, a) => { root.queue-action(i, a); }
                                clear-queue => { root.clear-queue(); }
                                toggle-autoplay => { root.toggle-autoplay(); }
                                related-open(s, c) => { root.open-related(s, c); }
                            }
                        }
                    }
                }

                // --- player bar ---
                Rectangle {
                    height: 76px;
                    background: Yt.bar;
                    // Anywhere on the bar but its buttons opens / closes the
                    // full-screen view (buttons sit on top and take their clicks).
                    TouchArea {
                        enabled: root.now-title != "";
                        mouse-cursor: self.enabled ? pointer : default;
                        clicked => { root.expanded = !root.expanded; keys.focus(); }
                    }
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
                                spacing: 8px;
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
                                    Text {
                                        text: root.now-artist;
                                        color: now-artist-touch.has-hover ? Yt.text : Yt.secondary;
                                        overflow: elide;
                                        now-artist-touch := TouchArea {
                                            mouse-cursor: pointer;
                                            clicked => { root.expanded = false; root.row-action(-1, "artist"); keys.focus(); }
                                        }
                                    }
                                }
                                if root.now-title != "" && !root.compact: IconButton {
                                    icon: "dislike";
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.dislike-now(); keys.focus(); }
                                }
                                if root.now-title != "" && !root.tiny: IconButton {
                                    icon: root.liked ? "heart" : "heart-outline";
                                    active: root.liked;
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.toggle-like(); keys.focus(); }
                                }
                                if root.now-title != "" && !root.tiny: IconButton {
                                    icon: "more";
                                    y: (parent.height - self.height) / 2;
                                    clicked => { root.menu-row = -1; root.menu-x = self.absolute-position.x; root.menu-y = self.absolute-position.y; row-menu.show(); }
                                }
                            }
                        }
                        // Right: volume, modes, sleep, expand
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
                            if !root.tiny: IconButton {
                                icon: "cast";
                                active: root.casting != "";
                                y: (parent.height - self.height) / 2;
                                clicked => { root.cast-scan(); cast-menu.show(); }
                            }
                            if !root.tiny: IconButton {
                                icon: "moon";
                                active: root.sleep-text != "";
                                y: (parent.height - self.height) / 2;
                                clicked => { sleep-menu.show(); }
                            }
                            if root.sleep-text != "" && !root.compact: Text { text: root.sleep-text; color: Yt.text; font-size: 12px; vertical-alignment: center; }
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

        // --- Google account dialog ---
        if root.account-open: Dialog {
            title: "Google account";
            card-width: 480px;
            close => { root.account-open = false; keys.focus(); }
            if root.signed-in: VerticalLayout {
                spacing: 12px;
                Text { text: "✓ Signed in. Your library syncs from YouTube; likes and playlist changes go back to it."; color: Yt.text; wrap: word-wrap; }
                Text { text: "OAuth client: " + root.client-id; color: Yt.secondary; font-size: 12px; overflow: elide; }
                HorizontalLayout {
                    alignment: start;
                    Pill { text: "Sign out"; icon: "account"; clicked => { root.sign-out(); } }
                }
            }
            if !root.signed-in: VerticalLayout {
                spacing: 10px;
                Text { text: "1. Your Google OAuth client"; color: Yt.text; font-weight: 600; }
                Text {
                    text: "In Google Cloud: enable YouTube Data API v3, add your account as a test user, create a client of type “Desktop app” and download its JSON (README: “Set up Google sign-in”).";
                    color: Yt.secondary;
                    wrap: word-wrap;
                }
                HorizontalLayout {
                    alignment: start;
                    spacing: 8px;
                    Pill { text: "Open Google Cloud"; icon: "open"; clicked => { root.open-console(); } }
                    if root.download-file != "": Pill {
                        text: "Import downloaded JSON";
                        icon: "playlist-add";
                        filled: true;
                        clicked => { root.import-downloaded(); }
                    }
                }
                Text { text: "…or paste it:"; color: Yt.secondary; font-size: 12px; }
                id-field := Field { placeholder: "Client ID (…apps.googleusercontent.com)"; text: root.client-id; }
                secret-field := Field { placeholder: "Client secret (GOCSPX-…)"; password: true; }
                HorizontalLayout {
                    alignment: start;
                    spacing: 12px;
                    Pill {
                        text: "Save client";
                        icon: "sync";
                        clicked => { root.save-client(id-field.text, secret-field.text); }
                    }
                    if root.client-id != "": Text { text: "✓ client set"; color: Yt.secondary; vertical-alignment: center; }
                }
                Rectangle { height: 1px; background: Yt.divider; }
                Text { text: "2. Sign in"; color: Yt.text; font-weight: 600; }
                HorizontalLayout {
                    alignment: start;
                    spacing: 12px;
                    Pill {
                        text: root.signing-in ? "Open the browser again" : "Sign in with Google";
                        icon: "account";
                        filled: root.client-id != "";
                        clicked => { root.sign-in(); }
                    }
                    if root.signing-in: Text { text: "Waiting for your browser…"; color: Yt.secondary; vertical-alignment: center; }
                }
                Text {
                    text: "Google warns the app is unverified: it's your own client, choose Continue.";
                    color: Yt.secondary;
                    font-size: 12px;
                    wrap: word-wrap;
                }
            }
            if root.status-text != "": Text {
                text: root.status-text;
                color: root.status-error ? #ff6b6b : Yt.secondary;
                font-size: 12px;
                wrap: word-wrap;
            }
        }

        // --- settings ---
        if root.settings-open: Dialog {
            title: "Settings";
            close => { root.settings-open = false; keys.focus(); }
            HorizontalLayout {
                spacing: 12px;
                VerticalLayout {
                    horizontal-stretch: 1;
                    alignment: center;
                    Text { text: "ytm-player " + root.app-version; color: Yt.text; }
                    Text {
                        text: root.update-version != "" ? "Version " + root.update-version + " is available" : "Up to date (checked at start)";
                        color: root.update-version != "" ? Yt.red : Yt.secondary;
                        font-size: 12px;
                    }
                }
                Pill {
                    text: root.updating ? "Updating…" : root.update-version != "" ? "Update" : "Check now";
                    icon: root.update-version != "" ? "download" : "sync";
                    filled: root.update-version != "";
                    y: (parent.height - self.height) / 2;
                    clicked => {
                        if (root.updating) { return; }
                        if (root.update-version != "") { root.update-now(); } else { root.check-updates(); }
                    }
                }
            }
            Toggle { text: "Autoplay: continue with similar songs"; on: root.autoplay; toggled => { root.toggle-autoplay(); } }
            Toggle { text: "Normalize volume (like YouTube Music)"; on: root.set-normalize; toggled => { root.setting-toggled("normalize_volume"); } }
            VerticalLayout {
                spacing: 4px;
                Text {
                    text: root.set-crossfade < 0.5 ? "Crossfade: off (gapless)" : "Crossfade: " + round(root.set-crossfade) + " s";
                    color: Yt.text;
                }
                ThinSlider {
                    width: 300px;
                    value: root.set-crossfade / 12;
                    changed(v) => { root.crossfade-changed(round(v * 12)); }
                }
            }
            Toggle { text: "Notify when the song changes"; on: root.set-notifications; toggled => { root.setting-toggled("notifications"); } }
            Toggle { text: "Icon in the menu bar / tray (next start)"; on: root.set-tray; toggled => { root.setting-toggled("tray"); } }
            HorizontalLayout {
                spacing: 12px;
                VerticalLayout {
                    horizontal-stretch: 1;
                    alignment: center;
                    Text { text: "Age-restricted songs"; color: Yt.text; }
                    Text {
                        text: "Use the YouTube sign-in of this browser, only for such songs. macOS: needs Full Disk Access for ytm-player (System Settings → Privacy & Security).";
                        color: Yt.secondary;
                        font-size: 12px;
                        wrap: word-wrap;
                    }
                }
                Pill {
                    text: root.set-cookies == "" ? "Off" : root.set-cookies;
                    icon: "account";
                    y: (parent.height - self.height) / 2;
                    clicked => { root.cookies-cycle(); }
                }
            }
            Text {
                text: "Last.fm and Discord: see config.toml (README: Integrations).";
                color: Yt.secondary;
                font-size: 12px;
                wrap: word-wrap;
            }
        }

        // --- new / rename playlist ---
        if root.name-dialog != 0: Dialog {
            title: root.name-dialog == 1 ? "New playlist" : "Rename playlist";
            close => { root.name-dialog = 0; keys.focus(); }
            name-field := Field { placeholder: "Name"; text: root.name-dialog == 2 ? root.tracks-title : ""; }
            HorizontalLayout {
                alignment: end;
                Pill {
                    text: root.name-dialog == 1 ? "Create" : "Rename";
                    filled: true;
                    icon: "check";
                    clicked => {
                        if (root.name-dialog == 1) { root.new-playlist(name-field.text); } else { root.rename-playlist(name-field.text); }
                        root.name-dialog = 0;
                        keys.focus();
                    }
                }
            }
        }

        if root.delete-dialog: Dialog {
            title: "Delete “" + root.tracks-title + "”?";
            close => { root.delete-dialog = false; keys.focus(); }
            Text { text: "The playlist is deleted on YouTube too."; color: Yt.secondary; wrap: word-wrap; }
            HorizontalLayout {
                alignment: end;
                spacing: 8px;
                Pill { text: "Cancel"; clicked => { root.delete-dialog = false; keys.focus(); } }
                Pill { text: "Delete"; icon: "delete"; filled: true; clicked => { root.delete-playlist(); root.delete-dialog = false; keys.focus(); } }
            }
        }

        // --- menus ---
        row-menu := PopupWindow {
            x: min(root.menu-x - 200px, root.win-width - 252px);
            y: min(root.menu-y, root.win-height - 300px);
            width: 240px;
            height: 296px;
            Rectangle {
                background: #282828;
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 140ms; easing: ease-out; }
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    padding: 6px;
                    MenuItem { icon: "radio"; text: "Start radio"; clicked => { root.row-action(root.menu-row, "radio"); } }
                    MenuItem { icon: "queue-next"; text: "Play next"; clicked => { root.row-action(root.menu-row, "next"); } }
                    MenuItem { icon: "queue-add"; text: "Add to queue"; clicked => { root.row-action(root.menu-row, "queue"); } }
                    MenuItem { icon: "playlist-add"; text: "Save to playlist"; clicked => { root.row-action(root.menu-row, "add"); add-popup.show(); } }
                    MenuItem { icon: "download"; text: "Download"; clicked => { root.row-action(root.menu-row, "download"); } }
                    MenuItem { icon: "dislike"; text: "Dislike"; clicked => { root.row-action(root.menu-row, "dislike"); } }
                    MenuItem { icon: "delete"; text: root.nav == 3 ? "Remove download" : "Remove from playlist"; clicked => { root.row-action(root.menu-row, "remove"); } }
                }
            }
        }

        playlist-menu := PopupWindow {
            x: root.win-width - 260px;
            y: 120px;
            width: 220px;
            height: 100px;
            Rectangle {
                background: #282828;
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 140ms; easing: ease-out; }
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    padding: 6px;
                    MenuItem { icon: "settings"; text: "Rename"; clicked => { root.name-dialog = 2; } }
                    MenuItem { icon: "delete"; text: "Delete"; clicked => { root.delete-dialog = true; } }
                }
            }
        }

        sleep-menu := PopupWindow {
            x: root.win-width - 240px;
            y: root.win-height - 76px - self.height;
            width: 220px;
            height: 252px;
            Rectangle {
                background: #282828;
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 140ms; easing: ease-out; }
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    padding: 6px;
                    Text { text: "Sleep timer"; color: Yt.text; font-weight: 600; height: 36px; vertical-alignment: center; x: 12px; }
                    for m in [15, 30, 45, 60]: MenuItem { icon: "moon"; text: m + " minutes"; clicked => { root.set-sleep(m); } }
                    MenuItem { icon: "moon"; text: "End of song"; clicked => { root.set-sleep(0); } }
                    if root.sleep-text != "": MenuItem { icon: "close"; text: "Turn off"; clicked => { root.set-sleep(-1); } }
                }
            }
        }

        cast-menu := PopupWindow {
            x: root.win-width - 300px;
            y: root.win-height - 76px - self.height;
            width: 280px;
            height: 96px + 44px * max(root.cast-devices.length, 1);
            Rectangle {
                background: #282828;
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 140ms; easing: ease-out; }
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    padding: 6px;
                    Text { text: "Play on"; color: Yt.text; font-weight: 600; height: 36px; vertical-alignment: center; x: 12px; }
                    MenuItem { icon: root.casting == "" ? "check" : "computer"; text: "This computer"; clicked => { root.cast-to(-1); } }
                    for d[i] in root.cast-devices: MenuItem {
                        icon: d == root.casting ? "check" : "cast";
                        text: d;
                        clicked => { root.cast-to(i); }
                    }
                    if root.cast-devices.length == 0: Text {
                        text: root.cast-scanning ? "Looking for Chromecasts…" : "No Chromecasts found";
                        color: Yt.secondary;
                        height: 44px;
                        vertical-alignment: center;
                        x: 12px;
                    }
                }
            }
        }

        add-popup := PopupWindow {
            x: root.win-width - min(300px, root.win-width - 20px);
            y: root.win-height - 76px - self.height;
            width: min(280px, root.win-width - 20px);
            height: min(root.win-height - 140px, 96px + 40px * max(root.add-choices.length, 1));
            Rectangle {
                background: #282828;
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 140ms; easing: ease-out; }
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    padding: 8px;
                    Text { text: "Save to playlist"; color: Yt.text; font-weight: 600; height: 36px; vertical-alignment: center; x: 8px; }
                    ListView {
                        for title[i] in root.add-choices: Rectangle {
                            height: 40px;
                            border-radius: 4px;
                            background: pick.has-hover ? Yt.raised : transparent;
                            Text { x: 8px; text: title; color: Yt.text; vertical-alignment: center; height: parent.height; }
                            pick := TouchArea { clicked => { root.add-to(i); add-popup.close(); } }
                        }
                    }
                    MenuItem { icon: "plus"; text: "New playlist…"; clicked => { root.add-to(-1); root.name-dialog = 1; } }
                }
            }
        }

        // Library picker for narrow windows (no sidebar).
        playlists-popup := PopupWindow {
            x: 12px;
            y: 150px;
            width: min(320px, root.win-width - 24px);
            height: min(root.win-height - 240px, 240px + 52px * root.playlists.length);
            Rectangle {
                background: #282828;
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 140ms; easing: ease-out; }
                border-radius: Yt.radius;
                drop-shadow-blur: 16px;
                drop-shadow-color: #00000099;
                VerticalLayout {
                    padding: 6px;
                    MenuItem { icon: "home"; text: "Home"; clicked => { root.nav-to(1); playlists-popup.close(); } }
                    if root.search-query != "": MenuItem { icon: "search"; text: "Search: " + root.search-query; clicked => { root.nav-to(6); playlists-popup.close(); } }
                    MenuItem { icon: "history"; text: "History"; clicked => { root.nav-to(2); playlists-popup.close(); } }
                    MenuItem { icon: "download"; text: "Downloads"; clicked => { root.nav-to(3); playlists-popup.close(); } }
                    MenuItem { icon: "album"; text: "Albums"; clicked => { root.nav-to(4); playlists-popup.close(); } }
                    MenuItem { icon: "artist"; text: "Artists"; clicked => { root.nav-to(5); playlists-popup.close(); } }
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
                opacity: 0;
                init => { self.opacity = 1; }
                animate opacity { duration: 140ms; easing: ease-out; }
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
