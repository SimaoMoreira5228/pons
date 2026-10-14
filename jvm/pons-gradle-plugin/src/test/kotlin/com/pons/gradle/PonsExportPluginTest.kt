package com.pons.gradle

import org.gradle.testkit.runner.GradleRunner
import org.gradle.testkit.runner.TaskOutcome
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File

class PonsExportPluginTest {
    @TempDir
    lateinit var projectDirectory: File

    @Test
    fun `prints the IR written by the bootRun application`() {
        settings()
        file(
            "build.gradle.kts",
            """
            plugins {
                java
                id("com.pons.export")
            }

            tasks.register<JavaExec>("bootRun") {
                mainClass.set("sample.Export")
                classpath = sourceSets["main"].runtimeClasspath
                jvmArgs("-Dfrom.boot=run")
                systemProperty("from.system", "kept")
            }
            """.trimIndent(),
        )
        file(
            "src/main/java/sample/Export.java",
            """
            package sample;

            import java.nio.file.Files;
            import java.nio.file.Path;

            public final class Export {
                public static void main(String[] args) throws Exception {
                    String path = System.getProperty("pons.export.path");
                    String exit = System.getProperty("pons.export.exit");
                    String boot = System.getProperty("from.boot");
                    String system = System.getProperty("from.system");
                    Files.createDirectories(Path.of(path).getParent());
                    Files.writeString(
                        Path.of(path),
                        "{\"exit\":\"" + exit + "\",\"boot\":\"" + boot + "\",\"system\":\"" + system + "\"}"
                    );
                }
            }
            """.trimIndent(),
        )

        val result = runner("ponsExport", "--quiet").build()

        assertEquals(
            "{\"exit\":\"true\",\"boot\":\"run\",\"system\":\"kept\"}",
            result.output.trim(),
        )
        assertEquals(TaskOutcome.SUCCESS, result.task(":ponsExport")?.outcome)
    }

    @Test
    fun `fails clearly without a bootRun task`() {
        settings()
        file(
            "build.gradle.kts",
            """
            plugins {
                java
                id("com.pons.export")
            }
            """.trimIndent(),
        )

        val result = runner("ponsExport").buildAndFail()

        assertTrue(result.output.contains("ponsExport needs a bootRun task"))
    }

    private fun runner(vararg arguments: String): GradleRunner =
        GradleRunner.create()
            .withProjectDir(projectDirectory)
            .withArguments(*arguments)
            .withPluginClasspath()

    private fun settings() {
        file("settings.gradle.kts", "rootProject.name = \"pons-export-sample\"")
    }

    private fun file(path: String, content: String) {
        projectDirectory.resolve(path).apply {
            parentFile.mkdirs()
            writeText(content)
        }
    }
}
