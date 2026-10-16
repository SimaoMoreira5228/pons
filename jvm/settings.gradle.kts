pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
    includeBuild("pons-gradle-plugin")
}

dependencyResolutionManagement {
    repositories {
        mavenCentral()
    }
}

rootProject.name = "pons-jvm"

include(":pons-ir-kotlin")
include(":pons-spring-boot-starter")
include(":conformance-reference")
