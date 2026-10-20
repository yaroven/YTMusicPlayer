package dev.yarek.music.ui

import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.draw.clip
import androidx.compose.ui.Alignment
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.foundation.Image
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
import dev.yarek.music.App
import dev.yarek.music.R

@Composable
fun Root() {
    val app = App.instance
    // Signed out: only the welcome screen (the app shows nothing but the
    // account's own music, never anonymous suggestions).
    if (!app.account.signedIn) {
        WelcomeScreen()
        return
    }
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
    // Covers and lyrics for songs downloaded before they were kept with them, once online.
    val online by app.network.online.collectAsState()
    LaunchedEffect(online) { if (online) library.fetchMissingExtras() }
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

/** Signed out: the app, what it shows (only the account's own music) and the sign-in. */
@Composable
private fun WelcomeScreen() {
    var signingIn by remember { mutableStateOf(false) }
    Column(
        Modifier.fillMaxSize().background(YT.background).systemBarsPadding().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.weight(1f))
        Image(painterResource(R.drawable.ic_launcher_foreground), null, Modifier.size(180.dp))
        Text("Yarek Music", fontSize = 32.sp, fontWeight = FontWeight.Bold, color = Color.White)
        Spacer(Modifier.height(8.dp))
        Text(stringResource(R.string.gate_title), fontSize = 18.sp, color = Color.White)
        Spacer(Modifier.height(12.dp))
        Text(
            stringResource(R.string.gate_text), fontSize = 15.sp, color = YT.secondary,
            textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 8.dp),
        )
        Spacer(Modifier.weight(1f))
        Box(
            Modifier.fillMaxWidth().height(52.dp).clip(CircleShape).background(Color.White)
                .clickable { signingIn = true },
            contentAlignment = Alignment.Center,
        ) {
            Text(stringResource(R.string.sign_in_google), color = Color.Black, fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
        }
    }
    if (signingIn) SignInDialog { signingIn = false }
}
