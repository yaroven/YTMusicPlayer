//! Typed YouTube Data API v3 client: `playlists.list?mine=true`,
//! `playlistItems.list`, `videos.list` (batched by 50 ids), liked videos via
//! `videos.list?myRating=like`. Sends `If-None-Match` with cached ETags so
//! unchanged resources return 304.
