package com.pons.spring

import com.fasterxml.jackson.databind.ObjectMapper
import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Document
import com.pons.ir.Info
import com.pons.ir.PonsIrJson
import kotlinx.serialization.json.Json
import org.springframework.context.ApplicationContext
import java.nio.file.Files
import java.nio.file.Path

class PonsExportException(
    val diagnostics: List<Diagnostic>,
) : IllegalStateException(
    diagnostics.joinToString("\n") { "${it.code}: ${it.message}" },
)

class PonsExporter(
    private val context: ApplicationContext,
    private val mapper: ObjectMapper,
    private val info: Info,
    private val defaultAuthScheme: String? = null,
    private val lowerers: List<PonsDocumentLowerer> = emptyList(),
) {
    fun document(): Document {
        val document = PonsDocumentBuilder(context, mapper, info, defaultAuthScheme).build()
        return lowerers.fold(document) { current, lowerer -> lowerer.lower(current) }
    }

    fun render(): String {
        val document = document()
        val errors = document.diagnostics.filter { it.severity == DiagnosticSeverity.Error }
        if (errors.isNotEmpty()) throw PonsExportException(errors)
        return json.encodeToString(Document.serializer(), document)
    }

    fun export(path: Path) {
        val target = path.toAbsolutePath()
        target.parent?.let(Files::createDirectories)
        Files.writeString(target, render())
    }
}

private val json = Json(PonsIrJson.instance) {
    prettyPrint = true
}
