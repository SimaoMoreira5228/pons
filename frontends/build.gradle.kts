plugins {
    base
    kotlin("jvm") version "2.3.0" apply false
    kotlin("plugin.serialization") version "2.3.0" apply false
    kotlin("plugin.spring") version "2.3.0" apply false
}

tasks.register("ponsGradlePluginTest") {
    dependsOn(gradle.includedBuild("pons-gradle-plugin").task(":test"))
}

tasks.named("build") {
    dependsOn("ponsGradlePluginTest")
}
