//! Slint UI definition. Kept declarative; all behaviour lives in `gui::run`.

slint::slint! {
    import { Button, LineEdit, ListView, Slider, AboutSlint, VerticalBox, HorizontalBox, Palette } from "std-widgets.slint";

    export struct PlaylistRow {
        title: string,
        count: int,
    }

    export struct TrackRow {
        num: string,
        title: string,
        artist: string,
        time: string,
        playing: bool,
    }

    global Theme {
        out property <color> accent: #e53935;
        out property <color> muted: #8a8a8a;
        out property <color> selected: #e5393533;
    }

    // A thin clickable bar: progress and seek.
    component Progress inherits Rectangle {
        in property <float> ratio;
        callback seek(float);
        height: 6px;
        border-radius: 3px;
        background: #8a8a8a44;
        Rectangle {
            x: 0;
            width: parent.width * clamp(root.ratio, 0, 1);
            border-radius: 3px;
            background: Theme.accent;
        }
        TouchArea {
            clicked => { root.seek(self.mouse-x / root.width); }
        }
    }

    // Transport buttons drawn as vector paths: no dependency on a font
    // having media glyphs (most system fonts lack ⏮ ⏸ ⏭).
    component IconButton inherits Rectangle {
        in property <string> icon; // "prev" | "play" | "pause" | "next" | "busy"
        callback clicked();
        width: 44px;
        height: 34px;
        border-radius: 6px;
        background: touch.pressed ? Theme.selected : touch.has-hover ? #8a8a8a33 : #8a8a8a1f;
        touch := TouchArea { clicked => { root.clicked(); } }
        Path {
            width: 16px;
            height: 16px;
            x: (parent.width - self.width) / 2;
            y: (parent.height - self.height) / 2;
            viewbox-width: 16;
            viewbox-height: 16;
            fill: Palette.foreground;
            commands: root.icon == "play" ? "M 3 1 L 14 8 L 3 15 Z"
                : root.icon == "pause" ? "M 3 1 L 6.5 1 L 6.5 15 L 3 15 Z M 9.5 1 L 13 1 L 13 15 L 9.5 15 Z"
                : root.icon == "prev" ? "M 2 1 L 4 1 L 4 15 L 2 15 Z M 14 1 L 14 15 L 5 8 Z"
                : root.icon == "next" ? "M 12 1 L 14 1 L 14 15 L 12 15 Z M 2 1 L 11 8 L 2 15 Z"
                : "M 1 7 L 4 7 L 4 9 L 1 9 Z M 6.5 7 L 9.5 7 L 9.5 9 L 6.5 9 Z M 12 7 L 15 7 L 15 9 L 12 9 Z";
        }
    }

    export component MainWindow inherits Window {
        title: "ytm-player";
        // The window's pixel buffers dominate GUI memory; keep the default modest.
        preferred-width: 900px;
        preferred-height: 580px;
        min-width: 640px;
        min-height: 400px;

        in property <[PlaylistRow]> playlists;
        in-out property <int> selected-playlist: -1;
        in property <[TrackRow]> tracks;
        in-out property <int> selected-track: -1;
        in property <string> tracks-title;

        in property <string> now-title: "Nothing playing";
        in property <string> now-artist;
        in property <bool> playing;
        in property <bool> loading;
        in property <float> progress;
        in property <string> position-text;
        in property <float> volume: 0.8;
        in property <bool> shuffle;
        in property <string> repeat-label: "Repeat";
        in property <bool> repeat-on;
        in property <bool> liked;
        in property <string> status-text;
        in property <bool> status-error;
        in property <bool> syncing;

        callback select-playlist(int);
        callback select-track(int);
        callback play-track(int);
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

        forward-focus: keys;
        keys := FocusScope {
            key-pressed(event) => {
                if (event.text == " ") { root.toggle-pause(); return accept; }
                if (event.text == "n") { root.next(); return accept; }
                if (event.text == "p") { root.prev(); return accept; }
                if (event.text == "f") { root.toggle-like(); return accept; }
                if (event.text == "/") { search.focus(); return accept; }
                reject
            }

            VerticalLayout {
                HorizontalLayout {
                    // --- sidebar ---
                    Rectangle {
                        width: 230px;
                        background: #8a8a8a14;
                        VerticalBox {
                            Text { text: "Library"; font-size: 16px; font-weight: 700; }
                            ListView {
                                for p[i] in root.playlists: Rectangle {
                                    height: 30px;
                                    border-radius: 4px;
                                    background: i == root.selected-playlist ? Theme.selected : transparent;
                                    HorizontalLayout {
                                        padding-left: 8px; padding-right: 8px;
                                        Text { text: p.title; vertical-alignment: center; overflow: elide; horizontal-stretch: 1; }
                                        Text { text: p.count; color: Theme.muted; vertical-alignment: center; }
                                    }
                                    TouchArea { clicked => { root.select-playlist(i); keys.focus(); } }
                                }
                            }
                            Button {
                                text: root.syncing ? "Syncing…" : "Sync library";
                                enabled: !root.syncing;
                                clicked => { root.sync(); }
                            }
                        }
                    }

                    // --- tracks ---
                    VerticalBox {
                        HorizontalLayout {
                            spacing: 8px;
                            Text { text: root.tracks-title; font-size: 16px; font-weight: 700; vertical-alignment: center; }
                            search := LineEdit {
                                placeholder-text: "Filter (/)";
                                edited(text) => { root.filter-changed(text); }
                                accepted => { keys.focus(); }
                            }
                        }
                        ListView {
                            for t[i] in root.tracks: Rectangle {
                                height: 30px;
                                border-radius: 4px;
                                background: i == root.selected-track ? Theme.selected : transparent;
                                HorizontalLayout {
                                    padding-left: 8px; padding-right: 8px; spacing: 12px;
                                    Text { text: t.num; width: 40px; horizontal-alignment: right; vertical-alignment: center; color: t.playing ? Theme.accent : Theme.muted; }
                                    Text { text: t.title; overflow: elide; horizontal-stretch: 3; vertical-alignment: center; color: t.playing ? Theme.accent : Palette.foreground; font-weight: t.playing ? 700 : 400; }
                                    Text { text: t.artist; overflow: elide; horizontal-stretch: 2; vertical-alignment: center; color: Theme.muted; }
                                    Text { text: t.time; width: 52px; horizontal-alignment: right; vertical-alignment: center; color: Theme.muted; }
                                }
                                TouchArea {
                                    clicked => { root.select-track(i); keys.focus(); }
                                    double-clicked => { root.play-track(i); }
                                }
                            }
                        }
                    }
                }

                // --- player bar ---
                Rectangle {
                    background: #8a8a8a14;
                    VerticalBox {
                        HorizontalLayout {
                            spacing: 8px;
                            IconButton { icon: "prev"; y: (parent.height - self.height) / 2; clicked => { root.prev(); keys.focus(); } }
                            IconButton {
                                icon: root.loading ? "busy" : root.playing ? "pause" : "play";
                                y: (parent.height - self.height) / 2;
                                clicked => { root.toggle-pause(); keys.focus(); }
                            }
                            IconButton { icon: "next"; y: (parent.height - self.height) / 2; clicked => { root.next(); keys.focus(); } }
                            VerticalLayout {
                                alignment: center;
                                horizontal-stretch: 1;
                                Text { text: root.now-title; font-weight: 700; overflow: elide; }
                                Text { text: root.now-artist; color: Theme.muted; overflow: elide; }
                            }
                            Button { text: root.liked ? "♥" : "♡"; clicked => { root.toggle-like(); keys.focus(); } }
                            Button { text: "Play next"; clicked => { root.play-next(); keys.focus(); } }
                            Button { text: "Add to…"; clicked => { add-popup.show(); } }
                            Button { text: "Shuffle"; checkable: true; checked: root.shuffle; clicked => { root.toggle-shuffle(); keys.focus(); } }
                            Button { text: root.repeat-label; checkable: true; checked: root.repeat-on; clicked => { root.cycle-repeat(); keys.focus(); } }
                            Text { text: "Vol"; vertical-alignment: center; color: Theme.muted; }
                            Slider {
                                width: 110px;
                                minimum: 0; maximum: 1;
                                value: root.volume;
                                changed(v) => { root.volume-changed(v); }
                            }
                        }
                        HorizontalLayout {
                            spacing: 10px;
                            Progress {
                                y: (parent.height - self.height) / 2;
                                horizontal-stretch: 1;
                                ratio: root.progress;
                                seek(r) => { root.seek(r); }
                            }
                            Text { text: root.position-text; color: Theme.muted; }
                        }
                        HorizontalLayout {
                            Text {
                                text: root.status-text;
                                color: root.status-error ? Theme.accent : Theme.muted;
                                overflow: elide;
                                horizontal-stretch: 1;
                            }
                            Text {
                                text: "Made with Slint";
                                color: Theme.muted;
                                TouchArea { clicked => { about-popup.show(); } }
                            }
                        }
                    }
                }
            }
        }

        add-popup := PopupWindow {
            x: (root.width - 320px) / 2;
            y: 60px;
            width: 320px;
            height: min(root.height - 120px, 40px + 32px * root.playlists.length);
            Rectangle {
                background: Palette.background;
                border-width: 1px;
                border-color: Theme.muted;
                border-radius: 8px;
                VerticalBox {
                    Text { text: "Add to playlist"; font-weight: 700; color: Palette.foreground; }
                    ListView {
                        for p[i] in root.playlists: Rectangle {
                            height: 30px;
                            // Index 0 is Liked music: use the heart instead.
                            visible: i > 0;
                            Text { x: 8px; text: p.title; color: Palette.foreground; vertical-alignment: center; height: parent.height; }
                            TouchArea { clicked => { root.add-to(i); add-popup.close(); } }
                        }
                    }
                }
            }
        }

        about-popup := PopupWindow {
            x: (root.width - 360px) / 2;
            y: (root.height - 260px) / 2;
            width: 360px;
            height: 260px;
            Rectangle {
                background: Palette.background;
                border-width: 1px;
                border-color: Theme.muted;
                border-radius: 8px;
                VerticalBox {
                    Text { text: "ytm-player"; font-size: 18px; font-weight: 700; color: Palette.foreground; }
                    AboutSlint {}
                }
            }
        }
    }
}
