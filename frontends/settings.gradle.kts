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
project(":pons-ir-kotlin").projectDir = file("../core/jvm/pons-ir-kotlin")
include(":pons-spring-boot-starter")
include(":conformance-reference")
project(":conformance-reference").projectDir = file("../conformance/spring-reference")
