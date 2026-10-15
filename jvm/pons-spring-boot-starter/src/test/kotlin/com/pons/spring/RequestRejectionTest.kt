package com.pons.spring

import com.fasterxml.jackson.databind.ObjectMapper
import com.networknt.schema.JsonSchema
import com.networknt.schema.JsonSchemaFactory
import com.networknt.schema.SpecVersion
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Direction
import com.pons.ir.Document
import com.pons.ir.Doc
import com.pons.ir.ErrorContent
import com.pons.ir.ErrorResponse
import com.pons.ir.ErrorStatus
import com.pons.ir.Field
import com.pons.ir.Info
import com.pons.ir.InlineType
import com.pons.ir.NamedRef
import com.pons.ir.ObjectOpenness
import com.pons.ir.Operation
import com.pons.ir.PonsIrJson
import com.pons.ir.Presence
import com.pons.ir.TypeDef
import com.pons.ir.TypeKind
import com.pons.ir.TypeRef
import com.pons.spring.testapp.RequestRejectionApplication
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test
import org.springframework.mock.web.MockServletContext
import org.springframework.web.context.support.AnnotationConfigWebApplicationContext
import java.io.File
import java.nio.file.Files

private val repoRoot: File =
    System.getProperty("pons.repoRoot")?.let(::File)
        ?: generateSequence(File("").absoluteFile) { it.parentFile }
            .first { File(it, "fixtures").isDirectory }

private val schema: JsonSchema by lazy {
    val source = Files.readString(File(repoRoot, "crates/pons-ir/schema/ir.schema.json").toPath())
    JsonSchemaFactory.getInstance(SpecVersion.VersionFlag.V202012).getSchema(source)
}

class RequestRejectionTest {
    private lateinit var context: AnnotationConfigWebApplicationContext
    private lateinit var document: Document

    @BeforeEach
    fun export() {
        context = AnnotationConfigWebApplicationContext()
        context.servletContext = MockServletContext()
        context.register(RequestRejectionApplication::class.java)
        context.refresh()

        val exporter = PonsExporter(
            context = context,
            mapper = context.getBean(ObjectMapper::class.java),
            info = Info(title = "Request Rejection", version = "1"),
        )
        val rendered = exporter.render()
        val messages = schema.validate(ObjectMapper().readTree(rendered))
        assertTrue(
            messages.isEmpty(),
            messages.joinToString("\n") { "${it.instanceLocation}: ${it.message}" },
        )
        document = PonsIrJson.instance.decodeFromString(Document.serializer(), rendered)
    }

    @AfterEach
    fun close() {
        context.close()
    }

    @Test
    fun everyValidatedOperationDeclaresTheRejectionItCanServe() {
        for (id in listOf("combined", "head")) {
            val rejection = rejection(operation(id))
            assertEquals(ErrorStatus.Exact(422), rejection.status)
            assertEquals(REJECTED_NAME, rejection.name)
            assertNull(rejection.mediaType)
            assertTrue(rejection.headers.isEmpty())
            assertEquals(false, rejection.retryable)
            assertEquals(
                Doc(summary = "The request does not satisfy the published request contract."),
                rejection.doc,
            )
        }
        val combined = rejection(operation("combined"))
        assertEquals(ErrorContent.Json, combined.content)
        assertEquals(REPORT_TYPE_ID, (combined.body as TypeRef.Named).ref.id)
        assertNull(rejectionOf(operation("plain")))
        assertNull(rejectionOf(operation("note")))
        for (id in listOf("combined", "head", "declared")) {
            assertEquals(1, operation(id).errors.count { it.status == ErrorStatus.Exact(422) })
        }
    }

    @Test
    fun theReportTypeIsPublishedOnceWithTheAxumShape() {
        val reports = document.types.filterValues { it.name == listOf("violation", "report") }
        assertEquals(1, reports.size)
        val (reportId, report) = reports.entries.single()
        assertEquals(REPORT_TYPE_ID, reportId)
        assertEquals(Direction.Output, report.direction)
        assertEquals(REPORT_TYPE_ID, (rejection(operation("combined")).body as TypeRef.Named).ref.id)

        val struct = report.definition as TypeKind.StructType
        assertEquals(ObjectOpenness.Reject, struct.openness)
        assertTrue(struct.flattened.isEmpty())
        val field = struct.fields.single()
        assertEquals("violations", field.wire)
        assertEquals(listOf("violations"), field.ident)
        assertEquals(Presence.Required, field.presence)
        assertNull(field.access)
        assertTrue(field.ext.isEmpty())
        assertEquals(Direction.Output, document.types.getValue(VIOLATION_TYPE_ID).direction)
        val array = (field.ty as TypeRef.Inline).type as InlineType.ArrayType
        assertEquals(TypeRef.Named(NamedRef(VIOLATION_TYPE_ID)), array.item)
        assertNull(array.unique)

        val violation = document.types.getValue(VIOLATION_TYPE_ID).definition as TypeKind.StructType
        assertEquals(ObjectOpenness.Reject, violation.openness)
        assertEquals(listOf("path", "reason"), violation.fields.map(Field::wire))
        for (published in violation.fields) {
            assertEquals(Presence.Required, published.presence)
            assertEquals(TypeRef.Inline(InlineType.StringType()), published.ty)
            assertTrue(published.ext.isEmpty())
        }
    }

    @Test
    fun aHeadOperationDeclaresTheRejectionWithoutABody() {
        val rejection = rejection(operation("head"))
        assertNull(rejection.body)
        assertEquals(ErrorContent.Empty, rejection.content)
        assertNull(rejection.mediaType)
    }

    @Test
    fun anOperationThatDeclaresItsOwnRejectionKeepsItAndEarnsAWarning() {
        val declared = operation("declared")
        val error = declared.errors.single()
        assertEquals(ErrorStatus.Exact(422), error.status)
        assertEquals(listOf("already", "rejected"), error.name)
        val warning = document.diagnostics.single { it.code == "SPRING-W012" }
        assertEquals(DiagnosticSeverity.Warning, warning.severity)
        assertTrue(warning.message.contains("already"))
    }

    @Test
    fun aBodyTheFrameworkReadsOnlyWarnsAboutTheRejectionsItCannotDeclare() {
        val note = operation("note")
        assertNull(rejectionOf(note))
        assertEquals(
            com.pons.ir.ContentKind.Text,
            note.body?.content,
        )
        val warning = document.diagnostics.single { it.code == "SPRING-W013" }
        assertTrue(warning.message.contains("400") && warning.message.contains("413"))
    }

    private fun operation(name: String): Operation =
        document.operations.single { it.id.lastOrNull()?.lastOrNull() == name }

    private fun rejectionOf(operation: Operation): ErrorResponse? =
        operation.errors.firstOrNull { it.status == ErrorStatus.Exact(422) }

    private fun rejection(operation: Operation): ErrorResponse =
        checkNotNull(rejectionOf(operation)) { "${operation.id} declares the request rejection" }
}
