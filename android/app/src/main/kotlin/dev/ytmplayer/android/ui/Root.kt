package dev.ytmplayer.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.LibraryMusic
import androidx.compose.material.icons.filled.Search
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
            containerColor = Color.Black,
            snackbarHost = { SnackbarHost(snackbar) },
            bottomBar = {
                Column {
                    MiniPlayer()
                    NavigationBar(containerColor = Color(0xFF111111)) {
                        listOf(
                            Triple(Tab.Home, Icons.Default.Home, R.string.home),
                            Triple(Tab.Search, Icons.Default.Search, R.string.search),
                            Triple(Tab.Library, Icons.Default.LibraryMusic, R.string.library),
                        ).forEach { (tab, icon, label) ->
                            NavigationBarItem(
                                selected = Router.tab == tab,
                                onClick = { Router.select(tab) },
                                icon = { Icon(icon, null) },
                                label = { Text(stringResource(label)) },
                            )
                        }
                    }
                }
            },
        ) { padding ->
            Column(Modifier.fillMaxSize().padding(padding)) {
                ConnectionBanner()
                Box(Modifier.weight(1f)) {
                    val stack = Router.stack(Router.tab)
                    val route = stack.lastOrNull()
                    // Each screen keeps its scroll position while others are on top.
                    states.SaveableStateProvider("${Router.tab}/${stack.size}/$route") {
                        when (route) {
                            null -> when (Router.tab) {
                                Tab.Home -> HomeScreen()
                                Tab.Search -> SearchScreen()
                                Tab.Library -> LibraryScreen()
                            }
                            is Route.Page -> PageScreen(route.id)
                            is Route.List -> TrackListScreen(route.list)
                            Route.AgeTest -> AgeTestScreen()
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
