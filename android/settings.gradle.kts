pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "yarek-music"
// core: YouTube Music, stream URLs, lyrics (plain Kotlin/JVM, tested on
// the desktop); app: the Android UI and player.
include(":core", ":app")
