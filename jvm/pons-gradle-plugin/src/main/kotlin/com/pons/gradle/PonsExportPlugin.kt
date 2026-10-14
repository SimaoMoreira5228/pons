package com.pons.gradle

import org.gradle.api.GradleException
import org.gradle.api.Plugin
import org.gradle.api.Project
import org.gradle.api.tasks.JavaExec
import java.io.ByteArrayOutputStream

class PonsExportPlugin : Plugin<Project> {
    override fun apply(project: Project) {
        val output = project.layout.buildDirectory.file("pons/ir.json")
        project.tasks.register("ponsExport", JavaExec::class.java) { task ->
            task.group = "pons"
            task.description = "Writes the Pons IR for this Spring application."
            task.dependsOn("classes")
            task.standardOutput = ByteArrayOutputStream()
            task.errorOutput = System.err
            task.doFirst {
                val bootRun = project.tasks.findByName("bootRun") as? JavaExec
                    ?: throw GradleException(
                        "ponsExport needs a bootRun task; apply the Spring Boot Gradle plugin",
                    )
                task.mainClass.set(bootRun.mainClass)
                task.classpath = bootRun.classpath
                task.jvmArgs = bootRun.jvmArgs
                task.systemProperties.putAll(bootRun.systemProperties)
                task.environment.putAll(bootRun.environment)
                task.workingDir = bootRun.workingDir
                task.systemProperty("pons.export.path", output.get().asFile.absolutePath)
                task.systemProperty("pons.export.exit", "true")
                task.systemProperty("spring.main.banner-mode", "off")
                task.systemProperty("logging.level.root", "OFF")
            }
            task.doLast {
                val file = output.get().asFile
                if (!file.isFile) {
                    throw GradleException("ponsExport did not write ${file.absolutePath}")
                }
                print(file.readText())
            }
        }
    }
}
