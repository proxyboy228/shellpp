package dev.shellpp.android

import androidx.compose.runtime.Composable

/** The Connections tab (reference-1 3, reference-2 6): built in 3.3 of run 15. */
@Composable
fun ConnectionsScreen(app: App, activity: MainActivity, nav: Nav) {
    Page(top = { TopBar("Connections") }) {}
}
