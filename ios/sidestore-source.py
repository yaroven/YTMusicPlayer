#!/usr/bin/env python3
"""Writes the SideStore / AltStore source for the iPhone app.

SideStore adds it once from the stable URL
https://github.com/<repo>/releases/latest/download/sidestore.json
and offers each new release's .ipa as an update from then on.

usage: sidestore-source.py <repo> <version> <build> <ipa> <out.json>
"""
import datetime
import json
import os
import sys

repo, version, build, ipa, out = sys.argv[1:6]
raw = f"https://raw.githubusercontent.com/{repo}/main"
description = (
    "Yarek Music, a lightweight YouTube Music player: search, albums, artists, radio, synced lyrics, "
    "background playback, lock screen and AirPlay. No account needed; liked songs and "
    "playlists stay on the phone."
)
source = {
    "name": "Yarek Music",
    # Kept from the "ytm-player" days: SideStore knows the source by it.
    "identifier": "dev.ytm-player.source",
    "subtitle": "A light YouTube Music player",
    "description": description,
    "iconURL": f"{raw}/ios/App/Resources/Assets.xcassets/AppIcon.appiconset/icon-1024.png",
    "website": f"https://github.com/{repo}",
    "tintColor": "#FF3B30",
    "apps": [
        {
            "name": "Yarek Music",
            "bundleIdentifier": "dev.yarek.music",
            "developerName": repo.split("/")[0],
            "subtitle": "A light YouTube Music player",
            "localizedDescription": description,
            "iconURL": f"{raw}/ios/App/Resources/Assets.xcassets/AppIcon.appiconset/icon-1024.png",
            "tintColor": "#FF3B30",
            "category": "entertainment",
            "screenshots": [],
            "versions": [
                {
                    "version": version,
                    "buildVersion": build,
                    "date": datetime.date.today().isoformat(),
                    "localizedDescription": f"https://github.com/{repo}/releases/tag/v{version}",
                    "downloadURL": f"https://github.com/{repo}/releases/download/v{version}/{os.path.basename(ipa)}",
                    "size": os.path.getsize(ipa),
                    "minOSVersion": "17.0",
                }
            ],
            "appPermissions": {
                "entitlements": [],
                "privacy": {},
                "backgroundModes": ["audio"],
            },
        }
    ],
    "news": [],
}
with open(out, "w") as f:
    json.dump(source, f, indent=2)
    f.write("\n")
