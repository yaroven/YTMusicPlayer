import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("org.jetbrains.kotlin.jvm")
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions { jvmTarget.set(JvmTarget.JVM_17) }
}

sourceSets {
    // yt-dlp-ejs' challenge solver, shared with the iPhone app.
    main { resources.srcDir("../../ios/YTMKit/Sources/YTMKit/Resources") }
}

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    // Android ships org.json; the JVM tests bring their own.
    compileOnly("org.json:json:20240303")
    testImplementation("org.json:json:20240303")
    testImplementation(kotlin("test"))
}

tasks.test {
    useJUnitPlatform()
    // `YTM_LIVE=1 ./gradlew :core:test` also runs the checks against YouTube / LRCLIB.
    environment("YTM_LIVE", System.getenv("YTM_LIVE") ?: "")
    testLogging {
        events("failed", "standard_out")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
    }
}
