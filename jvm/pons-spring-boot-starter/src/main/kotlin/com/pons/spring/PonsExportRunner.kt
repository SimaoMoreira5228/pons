package com.pons.spring

import org.springframework.boot.context.event.ApplicationReadyEvent
import org.springframework.context.event.EventListener
import java.nio.file.Path
import kotlin.system.exitProcess

class PonsExportRunner(
    private val exporter: PonsExporter,
    private val path: String,
    private val exit: Boolean,
) {
    @EventListener(ApplicationReadyEvent::class)
    fun export() {
        exporter.export(Path.of(path))
        if (exit) exitProcess(0)
    }
}
