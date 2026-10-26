package dev.yarek.music.ui

import androidx.compose.ui.text.style.TextAlign
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.ArrowDropDown
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.SystemUpdate
import androidx.compose.material.icons.filled.DownloadDone
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.ThumbUp
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Logout
import androidx.compose.material.icons.automirrored.filled.QueueMusic
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Code
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DownloadForOffline
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.MedicalServices
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.yarek.music.App
import dev.yarek.music.BuildConfig
import dev.yarek.music.Lang
import dev.yarek.music.LocalList
import dev.yarek.music.R
import dev.yarek.core.ItemKind
import kotlinx.coroutines.launch

/** What the library list shows: everything, or one kind. */
private enum class LibraryFilter(val label: Int) {
    Playlists(R.string.playlists), Songs(R.string.songs), Albums(R.string.albums), Artists(R.string.artists), Downloads(R.string.downloads),
}

/**
 * YouTube Music's Library: chips to narrow it, then liked music, downloads,
 * history, playlists and saved albums / artists as one list. Settings live
 * behind the avatar.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LibraryScreen() {
    val app = App.instance
    val library = app.library
    val account = app.account
    val scope = rememberCoroutineScope()
    var naming by remember { mutableStateOf(false) }
    var filter by rememberSaveable { mutableStateOf<LibraryFilter?>(null) }
    var byName by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(account.signedIn) {
        if (library.needsFirstSync) library.sync()
    }

    PullToRefreshBox(library.syncing, onRefresh = { scope.launch { library.sync() } }) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 96.dp)) {
            item {
                Row(Modifier.fillMaxWidth().statusBarsPadding().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.library), fontSize = 28.sp, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                    IconButton(onClick = { Router.open(Route.List(LocalList.History)) }) { Icon(Icons.Default.History, stringResource(R.string.history)) }
                    IconButton(onClick = { Router.open(Route.Search) }) { Icon(Icons.Default.Search, stringResource(R.string.search)) }
                    AvatarButton()
                }
                LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(LibraryFilter.entries) { f ->
                        ChipButton(stringResource(f.label), filter == f) { filter = if (filter == f) null else f }
                    }
                }
                // "Recent activity ▾": YouTube Music's order, or by name.
                Box(Modifier.padding(start = 8.dp, top = 8.dp)) {
                    var open by remember { mutableStateOf(false) }
                    Row(
                        Modifier.clip(CircleShape).clickable { open = true }.padding(horizontal = 8.dp, vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(stringResource(if (byName) R.string.a_to_z else R.string.recent_activity), fontSize = 15.sp, fontWeight = FontWeight.Medium)
                        Icon(Icons.Default.ArrowDropDown, null)
                    }
                    DropdownMenu(open, { open = false }) {
                        listOf(false to R.string.recent_activity, true to R.string.a_to_z).forEach { (value, label) ->
                            DropdownMenuItem(text = { Text(stringResource(label)) }, onClick = {
                                byName = value
                                open = false
                            })
                        }
                    }
                }
            }
            if (filter == null || filter == LibraryFilter.Songs) {
                item {
                    Entry(
                        stringResource(if (account.signedIn) R.string.liked_music else R.string.liked_songs),
                        stringResource(R.string.kind_count, stringResource(R.string.auto_playlist), songsCount(library.liked.size)), { LikedArt() },
                    ) { Router.open(Route.List(LocalList.Liked)) }
                }
            }
            if (filter == null || filter == LibraryFilter.Downloads) {
                item {
                    Entry(stringResource(R.string.downloads), songsCount(library.downloads.size), { IconArt(Icons.Default.DownloadForOffline) }) {
                        Router.open(Route.List(LocalList.Downloads))
                    }
                }
            }
            if (filter == null || filter == LibraryFilter.Playlists) {
                val playlists = if (byName) library.playlists.sortedBy { it.name.lowercase() } else library.playlists
                items(playlists, key = { it.id }) { p ->
                    Entry(p.name, stringResource(R.string.kind_count, stringResource(R.string.playlist), songsCount(p.tracks.size)), {
                        Artwork(p.tracks.firstOrNull()?.let { Art.Video(it.videoId) } ?: Art.None, 56.dp)
                    }) { Router.open(Route.List(LocalList.Playlist(p.id))) }
                }
            }
            val saved = (if (byName) library.saved.sortedBy { it.title.lowercase() } else library.saved).filter {
                when (filter) {
                    null -> true
                    LibraryFilter.Albums -> it.kind == ItemKind.Album
                    LibraryFilter.Artists -> it.kind == ItemKind.Artist
                    LibraryFilter.Playlists -> it.kind == ItemKind.Playlist
                    else -> false
                }
            }
            items(saved, key = { it.id }) { s ->
                var menu by remember { mutableStateOf(false) }
                Box {
                    ItemRow(dev.yarek.core.Item(s.kind, s.id, s.title, s.subtitle, s.thumbnail))
                    Box(Modifier.align(Alignment.CenterEnd)) {
                        IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, stringResource(R.string.more), tint = YT.secondary) }
                        DropdownMenu(menu, { menu = false }) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.remove_from_library)) },
                                leadingIcon = { Icon(Icons.Default.Delete, null) },
                                onClick = {
                                    menu = false
                                    library.deleteSaved(s.id)
                                },
                            )
                        }
                    }
                }
            }
        }
        // "+ New": a new playlist, as YouTube Music's Library offers it.
        Row(
            Modifier
                .align(Alignment.BottomEnd)
                .padding(16.dp)
                .height(48.dp)
                .clip(CircleShape)
                .background(Color.White)
                .clickable { naming = true }
                .padding(start = 16.dp, end = 24.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Default.Add, null, Modifier.size(24.dp), tint = Color.Black)
            Spacer(Modifier.width(12.dp))
            Text(stringResource(R.string.new_), color = Color.Black, fontWeight = FontWeight.Medium, fontSize = 15.sp)
        }
    }

    if (naming) {
        NameDialog(stringResource(R.string.new_playlist), "", { naming = false }) {
            library.createPlaylist(it)
            naming = false
        }
    }
}

/** "Liked music": YouTube Music's purple tile with a thumbs-up. */
@Composable
private fun LikedArt(size: androidx.compose.ui.unit.Dp = 56.dp) {
    Box(
        Modifier.size(size).clip(RoundedCornerShape(5.dp))
            .background(Brush.verticalGradient(listOf(Color(0xFF8C73F2), Color(0xFFE566BF)))),
        contentAlignment = Alignment.Center,
    ) { Icon(Icons.Default.ThumbUp, null, Modifier.size(size * 0.4f), tint = Color.White) }
}

@Composable
private fun IconArt(icon: androidx.compose.ui.graphics.vector.ImageVector, size: androidx.compose.ui.unit.Dp = 56.dp) {
    Box(Modifier.size(size).clip(RoundedCornerShape(5.dp)).background(YT.surface), contentAlignment = Alignment.Center) {
        Icon(icon, null, Modifier.size(size * 0.4f), tint = Color.White)
    }
}

/** A small grey heading over a part of a list. */
@Composable
private fun SectionTitle(text: String) {
    Text(text, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), fontSize = 14.sp, color = YT.secondary)
}

/** A library entry: picture, title, "N songs". */
@Composable
private fun Entry(title: String, subtitle: String, art: @Composable () -> Unit, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).height(72.dp).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        art()
        Spacer(Modifier.width(16.dp))
        Column {
            Text(title, fontSize = 16.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(subtitle, fontSize = 14.sp, color = YT.secondary)
        }
    }
}

/** The avatar's screen: the Google account, playback, language, about. */
@Composable
fun SettingsScreen() {
    val app = App.instance
    val library = app.library
    val account = app.account
    val player = app.player
    val scope = rememberCoroutineScope()
    var signingIn by remember { mutableStateOf(false) }
    var confirmSignOut by remember { mutableStateOf(false) }
    var choosingLanguage by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize()) {
        TopBar(stringResource(R.string.settings), back = true)
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
            item {
                Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(48.dp).clip(CircleShape)) {
                        val photo = account.photo
                        if (photo != null) {
                            coil.compose.AsyncImage(photo, null, Modifier.fillMaxSize(), contentScale = androidx.compose.ui.layout.ContentScale.Crop)
                        } else {
                            Icon(Icons.Default.AccountCircle, null, Modifier.fillMaxSize(), tint = YT.secondary)
                        }
                    }
                    Spacer(Modifier.width(14.dp))
                    Column {
                        Text(
                            account.name ?: stringResource(if (account.signedIn) R.string.signed_in_short else R.string.not_signed_in),
                            fontSize = 17.sp, fontWeight = FontWeight.SemiBold,
                        )
                        Text(stringResource(R.string.google_account), fontSize = 14.sp, color = YT.secondary)
                    }
                }
            }
            item {
                if (account.signedIn) {
                    SettingRow(Icons.Default.Sync, stringResource(R.string.sync_library), trailing = if (library.syncing) "…" else null) {
                        scope.launch { library.sync() }
                    }
                    SettingRow(Icons.AutoMirrored.Filled.Logout, stringResource(R.string.sign_out)) { confirmSignOut = true }
                } else {
                    SettingRow(Icons.Default.PersonAdd, stringResource(R.string.sign_in_google)) { signingIn = true }
                }
                SettingRow(Icons.Default.MedicalServices, stringResource(R.string.age_test_title)) { Router.open(Route.AgeTest) }
                Footnote(
                    when {
                        library.syncing -> stringResource(R.string.syncing)
                        account.signedIn && account.name != null -> stringResource(R.string.signed_in_as, account.name!!)
                        account.signedIn -> stringResource(R.string.signed_in)
                        else -> stringResource(R.string.sign_in_why)
                    },
                )
            }
            item { SectionTitle(stringResource(R.string.playback)) }
            item {
                Toggle(stringResource(R.string.autoplay), player.autoplay, player::changeAutoplay)
                Toggle(stringResource(R.string.normalize), player.normalize, player::changeNormalize)
                if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
                    var export by remember { mutableStateOf(app.settings.exportDownloads) }
                    Toggle(stringResource(R.string.export_downloads), export) {
                        export = it
                        app.settings.exportDownloads = it
                    }
                }
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Text(
                        if (player.crossfade == 0) stringResource(R.string.crossfade_off)
                        else stringResource(R.string.crossfade_seconds, player.crossfade),
                    )
                    Slider(player.crossfade.toFloat(), { player.changeCrossfade(it.toInt()) }, valueRange = 0f..12f, steps = 11, colors = ytSlider())
                }
                SettingRow(Icons.Default.Language, stringResource(R.string.language), Lang.label(LocalContext.current)) {
                    choosingLanguage = true
                }
            }
            item {
                val context = LocalContext.current
                SectionTitle(stringResource(R.string.about))
                SettingRow(Icons.Default.MusicNote, stringResource(R.string.version), BuildConfig.VERSION_NAME) {}
                val updater = app.updater
                SettingRow(
                    Icons.Default.SystemUpdate,
                    updater.available?.let { stringResource(R.string.update_to, it.version) } ?: stringResource(R.string.check_updates),
                    when {
                        updater.installing -> "…"
                        updater.upToDate -> stringResource(R.string.up_to_date)
                        else -> null
                    },
                ) { if (updater.available != null) updater.install() else updater.check() }
                SettingRow(Icons.Default.Code, stringResource(R.string.source_code)) {
                    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/yaroven/YTMusicPlayer")))
                }
            }
        }
    }
    if (signingIn) SignInDialog { signingIn = false }
    if (confirmSignOut) {
        AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            title = { Text(stringResource(R.string.sign_out_confirm)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmSignOut = false
                    account.signOut()
                }) { Text(stringResource(R.string.sign_out)) }
            },
            dismissButton = { TextButton(onClick = { confirmSignOut = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
    if (choosingLanguage) LanguageDialog { choosingLanguage = false }
}

@Composable
private fun LanguageDialog(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val current = Lang.current(context)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.language)) },
        text = {
            Column {
                listOf("" to stringResource(R.string.language_system), "en" to "English", "uk" to "Українська").forEach { (tag, label) ->
                    Row(
                        Modifier.fillMaxWidth().clickable {
                            onDismiss()
                            Lang.set(context, tag)
                        }.padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(current == tag, onClick = null)
                        Spacer(Modifier.width(12.dp))
                        Text(label)
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
fun SettingRow(icon: androidx.compose.ui.graphics.vector.ImageVector, title: String, trailing: String? = null, onClick: () -> Unit) {
    ListItem(
        headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        leadingContent = { Icon(icon, null) },
        trailingContent = trailing?.let { { Text(it, color = MaterialTheme.colorScheme.onSurfaceVariant) } },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable(onClick = onClick),
    )
}

@Composable
private fun Toggle(title: String, on: Boolean, onChange: (Boolean) -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        trailingContent = { Switch(on, onChange) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        modifier = Modifier.clickable { onChange(!on) },
    )
}

@Composable
private fun Footnote(text: String) {
    Text(
        text, Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** Liked songs, history, downloads or a playlist. */
@Composable
fun TrackListScreen(list: LocalList) {
    val app = App.instance
    val library = app.library
    val original = library.tracks(list)
    var sort by rememberSaveable { mutableStateOf(Sort.Default) }
    val tracks = sorted(original, sort, library.plays)
    var confirmDeleteAll by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var confirmDelete by remember { mutableStateOf(false) }
    val playlist = (list as? LocalList.Playlist)?.let { p -> library.playlists.firstOrNull { it.id == p.id } }
    Column(Modifier.fillMaxSize()) {
        TopBar("", back = true) {
            if (list == LocalList.History && tracks.isNotEmpty()) {
                TextButton(onClick = { library.clearHistory() }) { Text(stringResource(R.string.clear)) }
            }
            if (list == LocalList.Downloads && library.downloading.isNotEmpty()) {
                TextButton(onClick = { library.cancelDownloads() }) {
                    Icon(Icons.Default.Stop, null)
                    Text(stringResource(R.string.stop_downloads))
                }
            }
            if (list == LocalList.Downloads && tracks.isNotEmpty()) {
                TextButton(onClick = { confirmDeleteAll = true }) { Text(stringResource(R.string.delete_all)) }
            }
        }
        if (list == LocalList.Downloads) DownloadProgress()
        if (tracks.isEmpty()) {
            Message(
                Icons.Default.MusicNote, stringResource(R.string.no_songs),
                stringResource(if (list == LocalList.Downloads) R.string.no_downloads_hint else R.string.no_songs_hint),
            )
            return@Column
        }
        LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
            item {
                // YouTube Music's list header: the picture, the name, "N songs", ⬇ ▶ shuffle.
                Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    when (list) {
                        LocalList.Liked -> LikedArt(220.dp)
                        LocalList.Downloads -> IconArt(Icons.Default.DownloadForOffline, 220.dp)
                        LocalList.History -> IconArt(Icons.Default.History, 220.dp)
                        else -> Artwork(tracks.firstOrNull()?.let { Art.Video(it.videoId) } ?: Art.None, 220.dp)
                    }
                    Spacer(Modifier.height(20.dp))
                    Text(library.title(list), fontSize = 28.sp, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center)
                    Spacer(Modifier.height(6.dp))
                    Text(songsCount(tracks.size), fontSize = 14.sp, color = YT.secondary)
                    Spacer(Modifier.height(20.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                        if (list != LocalList.Downloads) DownloadAllButton(tracks)
                        CircleIcon(Icons.Default.PlayArrow, stringResource(R.string.play), 64.dp, filled = true) { playFrom(tracks, 0) }
                        Box {
                            var more by remember { mutableStateOf(false) }
                            CircleIcon(Icons.Default.MoreVert, stringResource(R.string.more)) { more = true }
                            DropdownMenu(more, { more = false }) {
                                DropdownMenuItem(
                                    text = { Text(stringResource(R.string.shuffle)) }, leadingIcon = { Icon(Icons.Default.Shuffle, null) },
                                    onClick = {
                                        more = false
                                        playFrom(tracks.shuffled(), 0)
                                    },
                                )
                                if (playlist != null) {
                                    DropdownMenuItem(
                                        text = { Text(stringResource(R.string.rename)) }, leadingIcon = { Icon(Icons.Default.Edit, null) },
                                        onClick = {
                                            more = false
                                            renaming = true
                                        },
                                    )
                                    DropdownMenuItem(
                                        text = { Text(stringResource(R.string.delete_playlist)) }, leadingIcon = { Icon(Icons.Default.Delete, null) },
                                        onClick = {
                                            more = false
                                            confirmDelete = true
                                        },
                                    )
                                }
                            }
                        }
                    }
                    Spacer(Modifier.height(16.dp))
                    LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        items(Sort.entries) { s ->
                            ChipButton(stringResource(s.label), sort == s) { sort = s }
                        }
                    }
                }
            }
            // Playlists that are on the phone whole, as YouTube Music lists them under Downloads.
            val whole = if (list == LocalList.Downloads) {
                library.playlists.filter { p -> p.tracks.isNotEmpty() && p.tracks.all { library.isDownloaded(it.videoId) } }
            } else {
                emptyList()
            }
            if (whole.isNotEmpty()) {
                item { SectionTitle(stringResource(R.string.downloaded_playlists)) }
                items(whole, key = { "playlist:" + it.id }) { p ->
                    Entry(p.name, stringResource(R.string.kind_count, stringResource(R.string.playlist), songsCount(p.tracks.size)), {
                        Artwork(p.tracks.firstOrNull()?.let { Art.Video(it.videoId) } ?: Art.None, 56.dp)
                    }) { Router.open(Route.List(LocalList.Playlist(p.id))) }
                }
                item { SectionTitle(stringResource(R.string.songs)) }
            }
            itemsIndexed(tracks) { i, track ->
                // Edits name the song's place in the list as stored, not as sorted.
                val at = if (sort == Sort.Default) i else original.indexOf(track)
                val extra = buildList {
                    if (playlist != null && sort == Sort.Default) {
                        addAll(
                            moveActions(
                                if (i > 0) ({ library.move(list, i, i - 1) }) else null,
                                if (i < tracks.size - 1) ({ library.move(list, i, i + 1) }) else null,
                                stringResource(R.string.move_up), stringResource(R.string.move_down),
                            ),
                        )
                    }
                    val label = when (list) {
                        LocalList.Liked -> null
                        LocalList.Downloads -> null
                        LocalList.History -> stringResource(R.string.remove_from_history)
                        is LocalList.Playlist -> stringResource(R.string.remove_from_playlist)
                    }
                    if (label != null) add(MenuAction(label, Icons.Default.Delete) { library.remove(at, list) })
                }
                TrackRow(track, tracks, i, extra = extra, plays = if (sort == Sort.MostPlayed) library.plays[track.videoId] ?: 0 else null)
            }
        }
    }
    if (confirmDeleteAll) {
        AlertDialog(
            onDismissRequest = { confirmDeleteAll = false },
            title = { Text(stringResource(R.string.delete_all_downloads_q)) },
            text = { Text(stringResource(R.string.delete_all_downloads_note)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmDeleteAll = false
                    library.removeAllDownloads()
                }) { Text(stringResource(R.string.delete)) }
            },
            dismissButton = { TextButton(onClick = { confirmDeleteAll = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
    if (renaming && playlist != null) {
        NameDialog(stringResource(R.string.rename), playlist.name, { renaming = false }) {
            library.rename(playlist.id, it)
            renaming = false
        }
    }
    if (confirmDelete && playlist != null) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(stringResource(R.string.delete_playlist_confirm, playlist.name)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = false
                    Router.back()
                    library.deletePlaylist(playlist.id)
                }) { Text(stringResource(R.string.delete)) }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

/** How a list of songs is ordered (as the desktop player sorts its lists). */
enum class Sort(val label: Int) { Default(R.string.sort_default), Title(R.string.sort_title), Artist(R.string.sort_artist), MostPlayed(R.string.sort_most_played) }

fun sorted(tracks: List<dev.yarek.core.Track>, sort: Sort, plays: Map<String, Int>): List<dev.yarek.core.Track> = when (sort) {
    Sort.Default -> tracks
    Sort.Title -> tracks.sortedBy { it.title.lowercase() }
    Sort.Artist -> tracks.sortedWith(compareBy({ it.artist.lowercase() }, { it.title.lowercase() }))
    Sort.MostPlayed -> tracks.sortedByDescending { plays[it.videoId] ?: 0 }
}

/** "Downloading 3 of 12" with a bar, while downloads run. */
@Composable
fun DownloadProgress() {
    val library = App.instance.library
    if (library.downloading.isEmpty() || library.batchTotal == 0) return
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
        Text(
            stringResource(R.string.downloading_of, minOf(library.batchDone + 1, library.batchTotal), library.batchTotal),
            fontSize = 13.sp, color = YT.secondary,
        )
        Spacer(Modifier.height(6.dp))
        androidx.compose.material3.LinearProgressIndicator(
            progress = { library.batchDone.toFloat() / library.batchTotal },
            modifier = Modifier.fillMaxWidth(), color = Color.White, trackColor = Color.White.copy(alpha = 0.15f),
        )
    }
}

/** Download every song of a list: ⬇, a spinner while it runs (tap stops), ✓ when all are on the phone. */
@Composable
fun DownloadAllButton(tracks: List<dev.yarek.core.Track>) {
    val library = App.instance.library
    val all = tracks.isNotEmpty() && tracks.all { library.isDownloaded(it.videoId) }
    val running = tracks.any { it.videoId in library.downloading }
    when {
        all -> CircleIcon(Icons.Default.DownloadDone, stringResource(R.string.downloaded)) {}
        running -> Box(contentAlignment = Alignment.Center) {
            CircularProgressIndicator(Modifier.size(48.dp), color = Color.White, strokeWidth = 2.dp)
            CircleIcon(Icons.Default.Stop, stringResource(R.string.stop_downloads), 40.dp) { library.cancelDownloads() }
        }
        else -> CircleIcon(Icons.Default.Download, stringResource(R.string.download_all), enabled = tracks.isNotEmpty()) {
            library.download(tracks)
        }
    }
}
