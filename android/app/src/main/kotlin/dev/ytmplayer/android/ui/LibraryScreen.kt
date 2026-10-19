package dev.ytmplayer.android.ui

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
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
import dev.ytmplayer.android.App
import dev.ytmplayer.android.BuildConfig
import dev.ytmplayer.android.Lang
import dev.ytmplayer.android.LocalList
import dev.ytmplayer.android.R
import dev.ytmplayer.core.ItemKind
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LibraryScreen() {
    val app = App.instance
    val library = app.library
    val account = app.account
    val player = app.player
    val scope = rememberCoroutineScope()
    var naming by remember { mutableStateOf(false) }
    var signingIn by remember { mutableStateOf(false) }
    var confirmSignOut by remember { mutableStateOf(false) }
    var choosingLanguage by remember { mutableStateOf(false) }

    LaunchedEffect(account.signedIn) {
        if (library.needsFirstSync) library.sync()
    }

    Column(Modifier.fillMaxSize()) {
        TopBar(stringResource(R.string.library))
        PullToRefreshBox(library.syncing, onRefresh = { scope.launch { library.sync() } }) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
                item {
                    Entry(
                        Icons.Default.Favorite,
                        stringResource(if (account.signedIn) R.string.liked_music else R.string.liked_songs),
                        library.liked.size.takeIf { it > 0 }?.toString(),
                    ) { Router.open(Route.List(LocalList.Liked)) }
                    Entry(Icons.Default.History, stringResource(R.string.history)) { Router.open(Route.List(LocalList.History)) }
                    Entry(
                        Icons.Default.DownloadForOffline, stringResource(R.string.downloads),
                        library.downloads.size.takeIf { it > 0 }?.toString(),
                    ) { Router.open(Route.List(LocalList.Downloads)) }
                }
                item { SectionTitle(stringResource(R.string.playlists)) }
                items(library.playlists, key = { it.id }) { p ->
                    Entry(Icons.AutoMirrored.Filled.QueueMusic, p.name, p.tracks.size.toString()) {
                        Router.open(Route.List(LocalList.Playlist(p.id)))
                    }
                }
                item { Entry(Icons.Default.Add, stringResource(R.string.new_playlist)) { naming = true } }
                if (library.saved.isNotEmpty()) {
                    item { SectionTitle(stringResource(R.string.saved_pages)) }
                    items(library.saved, key = { it.id }) { s ->
                        var menu by remember { mutableStateOf(false) }
                        val item = dev.ytmplayer.core.Item(s.kind, s.id, s.title, s.subtitle, s.thumbnail)
                        LaunchedEffect(s.id) { PageCache.note(item) }
                        Row(
                            Modifier.fillMaxWidth().prefetchOnPress(item).clickable { Router.open(Route.Page(s.id)) }
                                .padding(start = 16.dp, top = 6.dp, bottom = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Artwork(s.thumbnail?.let { Art.Url(it) } ?: Art.None, 48.dp, round = s.kind == ItemKind.Artist)
                            Spacer(Modifier.width(12.dp))
                            Column(Modifier.weight(1f)) {
                                Text(s.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(
                                    s.subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis,
                                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            IconButton(onClick = { menu = true }) {
                                Icon(Icons.Default.MoreVert, stringResource(R.string.more))
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

                item { SectionTitle(stringResource(R.string.google_account)) }
                item {
                    if (account.signedIn) {
                        Entry(Icons.Default.Sync, stringResource(R.string.sync_library), trailing = if (library.syncing) "…" else null) {
                            scope.launch { library.sync() }
                        }
                        Entry(Icons.AutoMirrored.Filled.Logout, stringResource(R.string.sign_out)) { confirmSignOut = true }
                    } else {
                        Entry(Icons.Default.PersonAdd, stringResource(R.string.sign_in_google)) { signingIn = true }
                    }
                    Entry(Icons.Default.MedicalServices, stringResource(R.string.age_test_title)) { Router.open(Route.AgeTest) }
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
                    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                        Text(
                            if (player.crossfade == 0) stringResource(R.string.crossfade_off)
                            else stringResource(R.string.crossfade_seconds, player.crossfade),
                        )
                        Slider(
                            player.crossfade.toFloat(), { player.changeCrossfade(it.toInt()) },
                            valueRange = 0f..12f, steps = 11,
                        )
                    }
                    Entry(Icons.Default.Language, stringResource(R.string.language), Lang.label(LocalContext.current)) {
                        choosingLanguage = true
                    }
                }

                item {
                    val context = LocalContext.current
                    SectionTitle(stringResource(R.string.about))
                    Entry(Icons.Default.MusicNote, stringResource(R.string.version), BuildConfig.VERSION_NAME) {}
                    Entry(Icons.Default.Code, stringResource(R.string.source_code)) {
                        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/yaroven/YTMusicPlayer")))
                    }
                }
            }
        }
    }

    if (naming) {
        NameDialog(stringResource(R.string.new_playlist), "", { naming = false }) {
            library.createPlaylist(it)
            naming = false
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
fun Entry(icon: androidx.compose.ui.graphics.vector.ImageVector, title: String, trailing: String? = null, onClick: () -> Unit) {
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
    val tracks = library.tracks(list)
    var menu by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var confirmDelete by remember { mutableStateOf(false) }
    val playlist = (list as? LocalList.Playlist)?.let { p -> library.playlists.firstOrNull { it.id == p.id } }
    Column(Modifier.fillMaxSize()) {
        TopBar(library.title(list), back = true) {
            if (list == LocalList.History && tracks.isNotEmpty()) {
                TextButton(onClick = { library.clearHistory() }) { Text(stringResource(R.string.clear)) }
            }
            if (list == LocalList.Downloads && library.downloading.isNotEmpty()) {
                TextButton(onClick = { library.cancelDownloads() }) {
                    Icon(Icons.Default.Stop, null)
                    Text(stringResource(R.string.stop_downloads))
                }
            }
            if (playlist != null) {
                IconButton(onClick = { menu = true }) {
                    Icon(Icons.Default.MoreVert, stringResource(R.string.more))
                    DropdownMenu(menu, { menu = false }) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.rename)) }, leadingIcon = { Icon(Icons.Default.Edit, null) },
                            onClick = {
                                menu = false
                                renaming = true
                            },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.delete_playlist)) }, leadingIcon = { Icon(Icons.Default.Delete, null) },
                            onClick = {
                                menu = false
                                confirmDelete = true
                            },
                        )
                    }
                }
            }
        }
        if (list == LocalList.Downloads && library.downloading.isNotEmpty()) {
            Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(Modifier.padding(end = 12.dp).width(16.dp), strokeWidth = 2.dp)
                Text(stringResource(R.string.downloading_n, library.downloading.size), style = MaterialTheme.typography.bodySmall)
            }
        }
        if (tracks.isEmpty()) {
            Message(
                Icons.Default.MusicNote, stringResource(R.string.no_songs),
                stringResource(if (list == LocalList.Downloads) R.string.no_downloads_hint else R.string.no_songs_hint),
            )
            return@Column
        }
        LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
            item {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Text(songsCount(tracks.size), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    PlayButtons(tracks)
                }
            }
            itemsIndexed(tracks) { i, track ->
                val extra = buildList {
                    if (playlist != null) {
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
                    if (label != null) add(MenuAction(label, Icons.Default.Delete) { library.remove(i, list) })
                }
                TrackRow(track, tracks, i, extra = extra)
            }
        }
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
