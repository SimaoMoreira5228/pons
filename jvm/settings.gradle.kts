pluginManagement {
    repositories {
        gradlePluginPortal()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    repositories {
        mavenCentral()
    }
}

rootProject.name = "pons-jvm"

include(":pons-ir-kotlin")
include(":pons-spring-boot-starter")
