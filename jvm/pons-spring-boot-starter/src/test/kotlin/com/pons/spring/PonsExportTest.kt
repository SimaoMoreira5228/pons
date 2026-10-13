package com.pons.spring

import com.fasterxml.jackson.databind.ObjectMapper
import com.networknt.schema.JsonSchema
import com.networknt.schema.JsonSchemaFactory
import com.networknt.schema.SpecVersion
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Document
import com.pons.ir.EnumRepr
import com.pons.ir.ErrorContent
import com.pons.ir.ErrorStatus
import com.pons.ir.Field
import com.pons.ir.Info
import com.pons.ir.NamedRef
import com.pons.ir.PonsIrJson
import com.pons.ir.Presence
import com.pons.ir.ResponseStatus
import com.pons.ir.TypeKind
import com.pons.ir.TypeRef
import com.pons.ir.VariantPayload
import com.pons.spring.testapp.AccountLocked
import com.pons.spring.testapp.LoginRequest
import com.pons.spring.testapp.Notification
import com.pons.spring.testapp.Role
import com.pons.spring.testapp.TestApplication
import com.pons.spring.testapp.User
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNotNull
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

class PonsExportTest {
    private lateinit var context: AnnotationConfigWebApplicationContext
    private lateinit var document: Document

    @BeforeEach
    fun export() {
        context = AnnotationConfigWebApplicationContext()
        context.servletContext = MockServletContext()
        context.register(TestApplication::class.java)
        context.refresh()

        val exporter = PonsExporter(
            context = context,
            mapper = context.getBean(ObjectMapper::class.java),
            info = Info(title = "Pons Test API", version = "1"),
        )
        val path = Files.createTempFile("pons-spring-ir", ".json")
        exporter.export(path)
        val json = Files.readString(path)

        val messages = schema.validate(ObjectMapper().readTree(json))
        assertTrue(
            messages.isEmpty(),
            messages.joinToString("\n") { "${it.instanceLocation}: ${it.message}" },
        )
        document = PonsIrJson.instance.decodeFromString(Document.serializer(), json)
    }

    @AfterEach
    fun close() {
        context.close()
    }

    @Test
    fun exportsTheDeclaredOperationsAndTypes() {
        assertTrue(document.diagnostics.none { it.severity == DiagnosticSeverity.Error })
        val operations = document.operations.associateBy { it.id.flatten().joinToString(".") }
        assertEquals(
            setOf("auth.login", "users.get", "notifications.latest"),
            operations.keys,
        )

        val login = operations.getValue("auth.login")
        assertEquals("POST", login.method.name.uppercase())
        assertEquals(
            TypeRef.Named(NamedRef(LoginRequest::class.java.name)),
            login.body?.ty,
        )
        assertEquals(ResponseStatus.Exact(200), login.responses.single().status)
        assertEquals("session", login.provides?.scheme)
        assertEquals(TypeRef.Named(NamedRef(InvalidCredentialsId)), login.errors.single().body)

        val users = operations.getValue("users.get")
        assertEquals(listOf(listOf("session")), users.auth.alternatives)
        val pathParameter = users.path[1] as com.pons.ir.PathSegment.Param
        assertEquals("id", pathParameter.param)
        assertEquals(64, (pathParameter.ty as TypeRef.Inline).type.let {
            (it as com.pons.ir.InlineType.IntType).bits
        })
    }

    @Test
    fun lowersJacksonOutputAndInputViews() {
        val user = document.types.getValue(User::class.java.name)
        val fields = (user.definition as TypeKind.StructType).fields.associateBy { it.wire }
        assertTrue("internalNote" !in fields)
        assertEquals("string", (fields.getValue("id").ty as TypeRef.Inline).type.let {
            (it as com.pons.ir.InlineType.IntType).wire.name.lowercase()
        })
        assertEquals(Presence.Optional, fields.getValue("nickname").presence)
        assertEquals(Presence.Optional, fields.getValue("deletedAt").presence)
        assertEquals(
            TypeRef.Inline(
                com.pons.ir.InlineType.StringType(format = com.pons.ir.FieldFormat.DateTime),
            ),
            fields.getValue("createdAt").ty,
        )
        assertEquals(
            TypeRef.Inline(com.pons.ir.InlineType.DecimalType(com.pons.ir.DecimalWire.Number)),
            fields.getValue("balance").ty,
        )

        val role = document.types.getValue(Role::class.java.name)
        val variants = (role.definition as TypeKind.EnumType).variants
        assertEquals(listOf("admin", "user"), variants.map { it.wire })

        val request = document.types.getValue(LoginRequest::class.java.name)
        val requestFields = (request.definition as TypeKind.StructType).fields.associateBy { it.wire }
        assertEquals(Presence.Optional, requestFields.getValue("rememberMe").presence)
    }

    @Test
    fun lowersQueryParametersAndTypedErrors() {
        val users = document.operations.first { it.id.flatten() == listOf("users", "get") }
        val query = users.query as TypeRef.Named
        val queryType = document.types.getValue(query.ref.id)
        val fields = (queryType.definition as TypeKind.StructType).fields.associateBy { it.wire }
        assertEquals(Presence.Optional, fields.getValue("verbose").presence)
        assertEquals(Presence.Optional, fields.getValue("role").presence)

        val login = document.operations.first { it.id.flatten() == listOf("auth", "login") }
        val error = login.errors.single()
        assertEquals(ErrorStatus.Exact(401), error.status)
        assertEquals(ErrorContent.Json, error.content)
        assertTrue(error.retryable == true)

        val common = document.commonErrors.single()
        assertEquals(ErrorStatus.Exact(423), common.status)
        assertEquals(TypeRef.Named(NamedRef(AccountLocked::class.java.name)), common.body)
    }

    @Test
    fun lowersJsonTypeInfoAndSubTypes() {
        val notification = document.types.getValue(Notification::class.java.name)
        val type = notification.definition as TypeKind.EnumType
        assertEquals(EnumRepr.InternalRepr("kind"), type.repr)
        assertEquals(listOf("email", "sms"), type.variants.map { it.wire })
        val email = type.variants.first { it.wire == "email" }
        val payload = email.payload as VariantPayload.StructPayload
        assertEquals(listOf("address"), payload.fields.map(Field::wire))
    }

    @Test
    fun declaresTheConfiguredAuthScheme() {
        val scheme = document.authSchemes.getValue("session")
        assertEquals("jwt", (scheme as com.pons.ir.AuthScheme.BearerScheme).format)
        assertNotNull(document.info.title)
    }

    private companion object {
        private val InvalidCredentialsId = com.pons.spring.testapp.InvalidCredentials::class.java.name
    }
}
