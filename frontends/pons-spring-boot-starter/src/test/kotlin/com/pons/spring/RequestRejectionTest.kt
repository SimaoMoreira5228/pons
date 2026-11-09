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
import com.pons.ir.PathSegment
import com.pons.ir.PonsIrJson
import com.pons.ir.Presence
import com.pons.ir.TypeDef
import com.pons.ir.TypeKind
import com.pons.ir.TypeRef
import com.pons.spring.testapp.RequestRejectionApplication
import kotlinx.serialization.json.JsonPrimitive
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test
import org.springframework.http.MediaType
import org.springframework.mock.web.MockServletContext
import org.springframework.test.web.servlet.MockMvc
import org.springframework.test.web.servlet.request.MockMvcRequestBuilders.post
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.content
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.jsonPath
import org.springframework.test.web.servlet.result.MockMvcResultMatchers.status
import org.springframework.test.web.servlet.setup.DefaultMockMvcBuilder
import org.springframework.test.web.servlet.setup.MockMvcBuilders
import org.springframework.web.context.support.AnnotationConfigWebApplicationContext
import java.io.File
import java.nio.file.Files

private val repoRoot: File =
    System.getProperty("pons.repoRoot")?.let(::File)
        ?: generateSequence(File("").absoluteFile) { it.parentFile }
            .first { File(it, "spec/fixtures").isDirectory }

private val schema: JsonSchema by lazy {
    val source = Files.readString(File(repoRoot, "spec/ir.schema.json").toPath())
    JsonSchemaFactory.getInstance(SpecVersion.VersionFlag.V202012).getSchema(source)
}

class RequestRejectionTest {
    private lateinit var context: AnnotationConfigWebApplicationContext
    private lateinit var document: Document
    private lateinit var mockMvc: MockMvc

    @BeforeEach
    fun export() {
        context = AnnotationConfigWebApplicationContext()
        context.servletContext = MockServletContext()
        context.register(RequestRejectionApplication::class.java)
        context.refresh()

        val mapper = context.getBean(ObjectMapper::class.java)
        val exporter = PonsExporter(
            context = context,
            mapper = mapper,
            info = Info(title = "Request Rejection", version = "1"),
            lowerers = listOf(PonsDocumentLowerer(::boundedFloats)),
        )
        val rendered = exporter.render()
        val messages = schema.validate(ObjectMapper().readTree(rendered))
        assertTrue(
            messages.isEmpty(),
            messages.joinToString("\n") { "${it.instanceLocation}: ${it.message}" },
        )
        document = PonsIrJson.instance.decodeFromString(Document.serializer(), rendered)
        val builder: DefaultMockMvcBuilder = MockMvcBuilders.webAppContextSetup(context)
            .addFilters(PonsRequestValidationFilter(exporter, mapper))
        mockMvc = builder.build()
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
            assertNull(rejection.media)
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
        assertNull(rejection.media)
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

    @Test
    fun aFloatBodyAcceptsItsInclusiveBoundsAndRejectsJustOutside() {
        mockMvc.perform(floatRequest(ratio = "0.0")).andExpect(status().isOk)
        mockMvc.perform(floatRequest(ratio = "1.0")).andExpect(status().isOk)

        mockMvc.perform(floatRequest(ratio = "-0.01"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(content().contentType(MediaType.APPLICATION_JSON))
            .andExpect(jsonPath("$.violations[0].path").value("/ratio"))
            .andExpect(jsonPath("$.violations[0].reason").value("number is below its minimum of 0.0"))

        mockMvc.perform(floatRequest(ratio = "1.01"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/ratio"))
            .andExpect(jsonPath("$.violations[0].reason").value("number is above its maximum of 1.0"))
    }

    @Test
    fun aFloatBodyKeepsTheExactJsonNumberItsBoundIsCheckedAgainst() {
        mockMvc.perform(floatRequest(ratio = "1.0000000000000001"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/ratio"))
            .andExpect(jsonPath("$.violations[0].reason").value("number is above its maximum of 1.0"))
    }

    @Test
    fun aFloatTextCarrierParsesWithBigDecimal() {
        mockMvc.perform(floatRequest(weight = "0.0000000000000000000000000000000000000001"))
            .andExpect(status().isOk)
    }

    @Test
    fun aFloatExclusiveBoundRejectsTheBoundary() {
        mockMvc.perform(floatRequest(headroom = "0.01")).andExpect(status().isOk)
        mockMvc.perform(floatRequest(headroom = "1.99")).andExpect(status().isOk)

        mockMvc.perform(floatRequest(headroom = "0.0"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/headroom"))
            .andExpect(
                jsonPath("$.violations[0].reason")
                    .value("number is not above its exclusive minimum of 0.0"),
            )

        mockMvc.perform(floatRequest(headroom = "2.0"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/headroom"))
            .andExpect(
                jsonPath("$.violations[0].reason")
                    .value("number is not below its exclusive maximum of 2.0"),
            )
    }

    @Test
    fun aFloatMultipleRejectsAValueOffTheStep() {
        mockMvc.perform(floatRequest(step = "0.25")).andExpect(status().isOk)
        mockMvc.perform(floatRequest(step = "0.75")).andExpect(status().isOk)

        mockMvc.perform(floatRequest(step = "0.3"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/step"))
            .andExpect(jsonPath("$.violations[0].reason").value("number is not a multiple of 0.25"))
    }

    @Test
    fun everyFloatCarrierUsesItsDeclaredBounds() {
        mockMvc.perform(floatRequest(scale = "10")).andExpect(status().isOk)
        mockMvc.perform(floatRequest(scale = "10.01"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/scale"))
            .andExpect(jsonPath("$.violations[0].reason").value("number is above its maximum of 10.0"))

        mockMvc.perform(floatRequest(weight = "0.01")).andExpect(status().isOk)
        mockMvc.perform(floatRequest(weight = "0.0"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/weight"))
            .andExpect(
                jsonPath("$.violations[0].reason")
                    .value("number is not above its exclusive minimum of 0.0"),
            )

        mockMvc.perform(floatRequest(offset = "1.0")).andExpect(status().isOk)
        mockMvc.perform(floatRequest(offset = "0.7"))
            .andExpect(status().`is`(REJECTED_STATUS))
            .andExpect(jsonPath("$.violations[0].path").value("/X-Offset"))
            .andExpect(jsonPath("$.violations[0].reason").value("number is not a multiple of 0.5"))
    }

    @Test
    fun aFloatRejectionCarriesTheSameViolationReportShapeAsOtherFields() {
        val response = mockMvc.perform(floatRequest(ratio = "-1.0")).andReturn().response
        assertEquals(REJECTED_STATUS, response.status)
        assertEquals(MediaType.APPLICATION_JSON_VALUE, response.contentType)
        val report = ObjectMapper().readTree(response.contentAsString)
        assertEquals(setOf("violations"), report.fieldNames().asSequence().toSet())
        val violations = report.get("violations")
        assertTrue(violations.isArray)
        assertEquals(1, violations.size())
        val violation = violations.single()
        assertEquals(setOf("path", "reason"), violation.fieldNames().asSequence().toSet())
        assertEquals("/ratio", violation.get("path").asText())
        assertEquals("number is below its minimum of 0.0", violation.get("reason").asText())
    }

    private fun floatRequest(
        ratio: String = "0.5",
        headroom: String = "0.5",
        step: String = "0.5",
        scale: String = "1",
        weight: String = "1",
        offset: String = "0.5",
    ) = post("/floats/$scale")
        .queryParam("weight", weight)
        .header("X-Offset", offset)
        .contentType(MediaType.APPLICATION_JSON)
        .content("""{"ratio":$ratio,"headroom":$headroom,"step":$step}""")

    private fun operation(name: String): Operation =
        document.operations.single { it.id.lastOrNull()?.lastOrNull() == name }

    private fun rejectionOf(operation: Operation): ErrorResponse? =
        operation.errors.firstOrNull { it.status == ErrorStatus.Exact(422) }

    private fun rejection(operation: Operation): ErrorResponse =
        checkNotNull(rejectionOf(operation)) { "${operation.id} declares the request rejection" }
}

private val floatBounds: Map<String, InlineType.FloatType> = mapOf(
    "ratio" to InlineType.FloatType(
        bits = 64,
        minimum = JsonPrimitive(0.0),
        maximum = JsonPrimitive(1.0),
    ),
    "headroom" to InlineType.FloatType(
        bits = 64,
        exclusiveMinimum = JsonPrimitive(0.0),
        exclusiveMaximum = JsonPrimitive(2.0),
    ),
    "step" to InlineType.FloatType(bits = 64, multipleOf = JsonPrimitive(0.25)),
    "scale" to InlineType.FloatType(
        bits = 64,
        minimum = JsonPrimitive(-10.0),
        maximum = JsonPrimitive(10.0),
    ),
    "weight" to InlineType.FloatType(bits = 64, exclusiveMinimum = JsonPrimitive(0.0)),
    "X-Offset" to InlineType.FloatType(bits = 64, multipleOf = JsonPrimitive(0.5)),
)

private fun boundedFloats(document: Document): Document {
    val types = document.types.mapValues { (_, definition) ->
        val struct = definition.definition as? TypeKind.StructType ?: return@mapValues definition
        definition.copy(
            definition = struct.copy(
                fields = struct.fields.map { field ->
                    field.copy(ty = boundedFloat(field.wire, field.ty))
                },
            ),
        )
    }
    val operations = document.operations.map { operation ->
        operation.copy(
            path = operation.path.map { segment ->
                when (segment) {
                    is PathSegment.Param -> segment.copy(ty = boundedFloat(segment.param, segment.ty))
                    is PathSegment.Literal -> segment
                }
            },
            headers = operation.headers.map { header ->
                header.copy(ty = boundedFloat(header.wire, header.ty))
            },
        )
    }
    return document.copy(types = types, operations = operations)
}

private fun boundedFloat(wire: String, ty: TypeRef): TypeRef {
    val bounds = floatBounds[wire] ?: return ty
    val inline = (ty as? TypeRef.Inline)?.type as? InlineType.FloatType ?: return ty
    return TypeRef.Inline(bounds.copy(bits = inline.bits))
}
