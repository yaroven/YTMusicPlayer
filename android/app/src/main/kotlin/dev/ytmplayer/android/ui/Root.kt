package dev.ytmplayer.android.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.collectAsState
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bookmark
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.outlined.BookmarkBorder
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Explore
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.material.icons.filled.Explore
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import dev.ytmplayer.android.App
import dev.ytmplayer.android.R

@Composable
fun Root() {
    val app = App.instance
    val player = app.player
    val library = app.library
    val snackbar = remember { SnackbarHostState() }
    val states = rememberSaveableStateHolder()

    // One-line notices and library failures (a like YouTube refused…).
    LaunchedEffect(Notice.id) {
        Notice.text?.let {
            snackbar.showSnackbar(it)
            Notice.clear()
        }
    }
    LaunchedEffect(app.updater.error) {
        app.updater.error?.let {
            snackbar.showSnackbar(it)
            app.updater.error = null
        }
    }
    // Covers for songs downloaded before they were kept with them, once online.
    val online by app.network.online.collectAsState()
    LaunchedEffect(online) { if (online) library.fetchMissingCovers() }
    LaunchedEffect(library.error) {
        library.error?.let {
            snackbar.showSnackbar(it)
            library.error = null
        }
    }

    BackHandler(enabled = player.showNowPlaying || Router.stack(Router.tab).isNotEmpty()) {
        if (player.showNowPlaying) player.showNowPlaying = false else Router.back()
    }

    Box(Modifier.fillMaxSize()) {
        Scaffold(
            containerColor = YT.background,
            snackbarHost = { SnackbarHost(snackbar) },
            bottomBar = {
                Column {
                    UpdateBanner()
                    MiniPlayer()
                    // YouTube Music's tab bar: black, outlined icons, filled and white when selected.
                    NavigationBar(containerColor = YT.background, tonalElevation = 0.dp) {
                        listOf(
                            Triple(Tab.Home, Icons.Filled.Home to Icons.Outlined.Home, R.string.home),
                            Triple(Tab.Explore, Icons.Filled.Explore to Icons.Outlined.Explore, R.string.explore),
                            Triple(Tab.Library, Icons.Filled.Bookmark to Icons.Outlined.BookmarkBorder, R.string.library),
                        ).forEach { (tab, icons, label) ->
                            val selected = Router.tab == tab
                            NavigationBarItem(
                                selected = selected,
                                onClick = { Router.select(tab) },
                                icon = { Icon(if (selected) icons.first else icons.second, null) },
                                label = { Text(stringResource(label), fontSize = 12.sp) },
                                colors = NavigationBarItemDefaults.colors(
                                    selectedIconColor = Color.White, selectedTextColor = Color.White,
                                    unselectedIconColor = YT.secondary, unselectedTextColor = YT.secondary,
                                    indicatorColor = Color.Transparent,
                                ),
                            )
                        }
                    }
                }
            },
        ) { padding ->
            // Screens draw under the status bar (their colours reach the top, as in
            // YouTube Music); each keeps its own content clear of it.
            Column(Modifier.fillMaxSize().padding(bottom = padding.calculateBottomPadding())) {
                ConnectionBanner()
                Box(Modifier.weight(1f)) {
                    val stack = Router.stack(Router.tab)
                    val route = stack.lastOrNull()
                    // Each screen keeps its scroll position while others are on top.
                    states.SaveableStateProvider("${Router.tab}/${stack.size}/$route") {
                        when (route) {
                            null -> when (Router.tab) {
                                Tab.Home -> HomeScreen()
                                Tab.Explore -> ExploreScreen()
                                Tab.Library -> LibraryScreen()
                            }
                            is Route.Page -> PageScreen(route.id)
                            is Route.List -> TrackListScreen(route.list)
                            Route.AgeTest -> AgeTestScreen()
                            Route.Settings -> SettingsScreen()
                            Route.Search -> SearchScreen()
                        }
                    }
                }
            }
        }
        AnimatedVisibility(
            player.showNowPlaying,
            enter = slideInVertically { it },
            exit = slideOutVertically { it },
        ) { NowPlaying() }
    }
}
