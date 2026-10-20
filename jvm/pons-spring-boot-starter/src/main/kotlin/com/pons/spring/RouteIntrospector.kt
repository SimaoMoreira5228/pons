package com.pons.spring

import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.annotation.JsonSerialize
import com.fasterxml.jackson.databind.ser.std.ToStringSerializer
import com.pons.ir.Body as IrBody
import com.pons.ir.ContentKind
import com.pons.ir.CredentialProvision
import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Direction
import com.pons.ir.Doc
import com.pons.ir.Field as IrField
import com.pons.ir.FieldDefault
import com.pons.ir.HeaderParameter
import com.pons.ir.InlineType
import com.pons.ir.Method as IrMethod
import com.pons.ir.OperationSemantics
import com.pons.ir.Origin
import com.pons.ir.PathSegment
import com.pons.ir.Presence
import com.pons.ir.Response as IrResponse
import com.pons.ir.ResponseStatus as IrResponseStatus
import com.pons.ir.TypeRef
import kotlinx.serialization.json.JsonPrimitive
import org.springframework.core.DefaultParameterNameDiscoverer
import org.springframework.core.ResolvableType
import org.springframework.core.annotation.AnnotatedElementUtils
import org.springframework.http.HttpEntity
import org.springframework.http.ProblemDetail
import org.springframework.http.ResponseEntity
import org.springframework.web.bind.annotation.CookieValue
import org.springframework.web.bind.annotation.MatrixVariable
import org.springframework.web.bind.annotation.ModelAttribute
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.RequestAttribute
import org.springframework.web.bind.annotation.RequestBody
import org.springframework.web.bind.annotation.RequestHeader
import org.springframework.web.bind.annotation.RequestMethod
import org.springframework.web.bind.annotation.RequestParam
import org.springframework.web.bind.annotation.RequestPart
import org.springframework.web.bind.annotation.ResponseStatus
import org.springframework.web.bind.annotation.SessionAttribute
import org.springframework.web.bind.annotation.ValueConstants
import org.springframework.web.method.HandlerMethod
import org.springframework.web.multipart.MultipartFile
import org.springframework.web.servlet.mvc.method.RequestMappingInfo
import org.springframework.web.servlet.mvc.method.annotation.RequestMappingHandlerMapping
import java.math.BigDecimal
import java.math.BigInteger
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.ZonedDateTime
import java.util.Date
import java.util.UUID

private val frameworkControllers = setOf(
    "org.springframework.boot.autoconfigure.web.servlet.error.BasicErrorController",
)

private val ignoredParameterTypes = setOf(
    "jakarta.servlet.ServletRequest",
    "jakarta.servlet.ServletResponse",
    "jakarta.servlet.http.HttpServletRequest",
    "jakarta.servlet.http.HttpServletResponse",
    "org.springframework.web.context.request.WebRequest",
    "org.springframework.web.context.request.NativeWebRequest",
    "java.security.Principal",
    "org.springframework.security.core.Authentication",
    "java.util.Locale",
    "java.util.TimeZone",
    "java.time.ZoneId",
    "org.springframework.ui.Model",
    "org.springframework.ui.ModelMap",
    "org.springframework.validation.BindingResult",
    "org.springframework.validation.Errors",
    "org.springframework.web.bind.support.SessionStatus",
    "org.springframework.web.util.UriComponentsBuilder",
    "org.springframework.http.HttpHeaders",
    "org.springframework.http.HttpMethod",
)

private val ignoredParameterAnnotations = setOf(
    "org.springframework.security.core.annotation.AuthenticationPrincipal",
)

internal class RouteIntrospector(
    private val mapper: ObjectMapper,
    private val types: JacksonTypes,
    private val auth: AuthIntrospector,
    private val errors: ErrorIntrospector,
    private val diagnostics: MutableList<Diagnostic>,
    private val parameterNameDiscoverer: DefaultParameterNameDiscoverer = DefaultParameterNameDiscoverer(),
) {
    fun lower(mapping: RequestMappingHandlerMapping): List<com.pons.ir.Operation> =
        mapping.handlerMethods.flatMap { (info, handler) -> lower(info, handler) }

    private fun lower(info: RequestMappingInfo, handler: HandlerMethod): List<com.pons.ir.Operation> {
        if (ignored(handler)) return emptyList()
        val patterns = info.patternValues.toList()
        if (patterns.isEmpty()) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E001",
                message = "the handler ${handler} has no request path",
            )
            return emptyList()
        }
        if (patterns.size > 1) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Warning,
                code = "SPRING-W001",
                message = "the handler ${handler} maps ${patterns.size} paths; only ${patterns.first()} is recorded",
            )
        }
        val methods = info.methodsCondition.methods
        if (methods.isEmpty()) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E002",
                message = "the handler ${handler} accepts every HTTP method; declare one so it can be exported",
            )
            return emptyList()
        }
        return methods.mapNotNull { requestMethod ->
            lowerOne(patterns.first(), requestMethod, info, handler)
        }
    }

    private fun lowerOne(
        pattern: String,
        requestMethod: RequestMethod,
        info: RequestMappingInfo,
        handler: HandlerMethod,
    ): com.pons.ir.Operation? {
        val method = irMethod(requestMethod) ?: run {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E003",
                message = "the HTTP method $requestMethod is not represented by the IR",
            )
            return null
        }
        val id = operationId(handler)
        val pathParameters = linkedMapOf<String, TypeRef>()
        val queryFields = mutableListOf<IrField>()
        val headers = mutableListOf<HeaderParameter>()
        var body: IrBody? = null
        var modelQuery: TypeRef? = null
        var validatedInput = false
        val requestContent = requestContent(info)

        for (parameter in handler.methodParameters) {
            if (parameter.parameterType.name in ignoredParameterTypes) continue
            if (parameter.parameterAnnotations.any {
                    it.annotationClass.qualifiedName in ignoredParameterAnnotations
                }
            ) {
                continue
            }
            if (parameter.parameterType.name == "kotlin.coroutines.Continuation") {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E004",
                    message = "the suspend handler ${handler} is not supported by this slice",
                )
                continue
            }
            val shape = kotlinMethodShape(handler.method, parameter.parameterIndex)
            when {
                parameter.hasParameterAnnotation(PathVariable::class.java) -> {
                    validatedInput = true
                    val annotation = parameter.getParameterAnnotation(PathVariable::class.java)!!
                    val name = parameterName(parameter, annotation.name.ifEmpty { annotation.value })
                    if (shape.nullable) {
                        diagnostics += Diagnostic(
                            severity = DiagnosticSeverity.Error,
                            code = "SPRING-E005",
                            message = "the path parameter $name is nullable, and a path segment has no null encoding",
                        )
                    }
                    if (!scalar(parameter.parameterType)) {
                        diagnostics += Diagnostic(
                            severity = DiagnosticSeverity.Error,
                            code = "SPRING-E006",
                            message = "the path parameter $name uses ${parameter.parameterType.name}; this slice only lowers scalar path parameters",
                        )
                    }
                    pathParameters[name] = parameterRef(parameter, shape, Direction.Input)
                }
                parameter.hasParameterAnnotation(RequestParam::class.java) -> {
                    validatedInput = true
                    val annotation = parameter.getParameterAnnotation(RequestParam::class.java)!!
                    val name = parameterName(parameter, annotation.name.ifEmpty { annotation.value })
                    val hasDefault = annotation.defaultValue != ValueConstants.DEFAULT_NONE
                    if (annotation.required && shape.nullable) {
                        diagnostics += Diagnostic(
                            severity = DiagnosticSeverity.Error,
                            code = "SPRING-E006",
                            message = "the required query parameter $name is nullable, so the contract cannot say when it is present",
                        )
                    }
                    if (!scalar(parameter.parameterType)) {
                        diagnostics += Diagnostic(
                            severity = DiagnosticSeverity.Error,
                            code = "SPRING-E007",
                            message = "the query parameter $name uses ${parameter.parameterType.name}; this slice only lowers scalar query fields",
                        )
                    }
                    val required = annotation.required && !shape.nullable && !hasDefault
                    queryFields += IrField(
                        wire = name,
                        ident = words(name.removePrefix("X-")),
                        ty = parameterRef(parameter, shape, Direction.Input),
                        presence = if (required) Presence.Required else Presence.Optional,
                        default = if (hasDefault) {
                            FieldDefault.Value(JsonPrimitive(annotation.defaultValue))
                        } else {
                            FieldDefault.Unspecified
                        },
                    )
                }
                parameter.hasParameterAnnotation(RequestHeader::class.java) -> {
                    validatedInput = true
                    val annotation = parameter.getParameterAnnotation(RequestHeader::class.java)!!
                    val name = parameterName(parameter, annotation.name.ifEmpty { annotation.value })
                    val required = annotation.required && !shape.nullable
                    headers += HeaderParameter(
                        wire = name,
                        ident = words(name.removePrefix("X-")),
                        ty = parameterRef(parameter, shape, Direction.Input),
                        presence = if (required) Presence.Required else Presence.Optional,
                    )
                }
                parameter.hasParameterAnnotation(RequestBody::class.java) -> {
                    val annotation = parameter.getParameterAnnotation(RequestBody::class.java)!!
                    val ty = parameterRef(parameter, shape, Direction.Input)
                    if (validatedBody(requestContent.kind, ty)) validatedInput = true
                    if (body != null) {
                        diagnostics += Diagnostic(
                            severity = DiagnosticSeverity.Error,
                            code = "SPRING-E008",
                            message = "the handler ${handler} declares more than one request body",
                        )
                    } else {
                        body = IrBody(
                            ty = ty,
                            content = requestContent.kind,
                            required = annotation.required && !shape.nullable,
                            mediaType = requestContent.mediaType,
                        )
                    }
                }
                parameter.hasParameterAnnotation(ModelAttribute::class.java) -> {
                    validatedInput = true
                    if (queryFields.isNotEmpty() || modelQuery != null) {
                        diagnostics += Diagnostic(
                            severity = DiagnosticSeverity.Error,
                            code = "SPRING-E009",
                            message = "the handler ${handler} mixes a model attribute with other query parameters; this slice lowers one query carrier",
                        )
                    } else {
                        modelQuery = parameterRef(parameter, shape, Direction.Input)
                    }
                }
                parameter.hasParameterAnnotation(RequestPart::class.java) ||
                    MultipartFile::class.java.isAssignableFrom(parameter.parameterType) -> {
                    diagnostics += Diagnostic(
                        severity = DiagnosticSeverity.Error,
                        code = "SPRING-E010",
                        message = "the handler ${handler} reads a multipart part, which this slice does not lower",
                    )
                }
                parameter.hasParameterAnnotation(CookieValue::class.java) ||
                    parameter.hasParameterAnnotation(SessionAttribute::class.java) ||
                    parameter.hasParameterAnnotation(RequestAttribute::class.java) ||
                    parameter.hasParameterAnnotation(MatrixVariable::class.java) -> {
                    diagnostics += Diagnostic(
                        severity = DiagnosticSeverity.Error,
                        code = "SPRING-E011",
                        message = "the handler ${handler} reads a non-query parameter source that this slice does not lower",
                    )
                }
                else -> {
                    diagnostics += Diagnostic(
                        severity = DiagnosticSeverity.Error,
                        code = "SPRING-E012",
                        message = "the parameter ${parameter.parameterName ?: parameter.parameterIndex} of ${handler} has no supported binding annotation",
                    )
                }
            }
        }

        val query = queryType(id, handler, queryFields, modelQuery)
        val path = path(pattern, pathParameters)
        val meta = meta(handler)
        val responses = response(handler, info)
        var declaredErrors = errors.handlerErrors(handler)
        if (validatedInput) {
            requestRejection(method, declaredErrors, diagnostics)?.let { rejection ->
                types.publishViolationReport()
                declaredErrors = declaredErrors + rejection
            }
        }
        body?.let { declareFrameworkRejections(it, responses, declaredErrors, diagnostics) }
        return com.pons.ir.Operation(
            id = id,
            method = method,
            path = path,
            query = query,
            headers = headers.sortedBy { it.wire },
            body = body,
            responses = responses,
            errors = declaredErrors,
            errorsInherit = true,
            auth = auth.requirement(handler),
            provides = meta.provides,
            semantics = OperationSemantics(
                safe = method == IrMethod.Get || method == IrMethod.Head || method == IrMethod.Options,
                idempotent = method != IrMethod.Post && method != IrMethod.Patch,
            ),
            doc = meta.doc,
            origin = Origin(source = "${handler.beanType.name}#${handler.method.name}"),
        )
    }

    private fun operationId(handler: HandlerMethod): List<List<String>> {
        val annotation = AnnotatedElementUtils.findMergedAnnotation(
            handler.method,
            PonsOperation::class.java,
        )
        val explicit = annotation?.name?.takeIf { it.isNotEmpty() }
        if (explicit != null) {
            return runCatching { operationId(explicit) }.getOrElse {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E013",
                    message = "the operation id $explicit on ${handler} is not a dotted name",
                )
                inferredOperationId(handler.beanType, handler.method)
            }
        }
        return inferredOperationId(handler.beanType, handler.method)
    }

    private fun queryType(
        id: List<List<String>>,
        handler: HandlerMethod,
        fields: List<IrField>,
        model: TypeRef?,
    ): TypeRef? {
        if (model != null) {
            if (fields.isNotEmpty()) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E021",
                    message = "the handler ${handler.beanType.name}.${handler.method.name} mixes a model attribute with explicit query fields",
                )
            }
            if (model !is TypeRef.Named) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E022",
                    message = "the model attribute of ${handler.beanType.name}.${handler.method.name} is not an object type",
                )
            }
            return model
        }
        if (fields.isEmpty()) return null
        val typeId = generatedTypeId(handler.beanType, handler.method, "query")
        types.define(
            typeId,
            com.pons.ir.TypeDef(
                name = id.flatten() + "query",
                direction = Direction.Input,
                definition = com.pons.ir.TypeKind.StructType(
                    fields = fields.sortedBy { it.wire },
                    openness = com.pons.ir.ObjectOpenness.Reject,
                ),
                origin = "${handler.beanType.name}#${handler.method.name}",
            ),
        )
        return TypeRef.Named(com.pons.ir.NamedRef(typeId))
    }

    private fun path(pattern: String, parameters: Map<String, TypeRef>): List<PathSegment> {
        if (pattern.isEmpty() || pattern == "/") return emptyList()
        return pattern.removePrefix("/").split("/").map { segment ->
            if (segment.length >= 2 && segment.startsWith("{") && segment.endsWith("}")) {
                val inner = segment.substring(1, segment.length - 1)
                val wildcard = inner.startsWith("*")
                val name = inner.substringBefore(':').removePrefix("*")
                if (wildcard) {
                    diagnostics += Diagnostic(
                        severity = DiagnosticSeverity.Error,
                        code = "SPRING-E014",
                        message = "the path $pattern has the wildcard parameter $name, which the IR cannot represent",
                    )
                }
                val type = parameters[name]
                if (type == null) {
                    diagnostics += Diagnostic(
                        severity = DiagnosticSeverity.Warning,
                        code = "SPRING-W002",
                        message = "the path $pattern has no bound parameter for $name, so it is recorded as a string",
                    )
                }
                PathSegment.Param(name, type ?: stringType())
            } else if (segment == "*" || segment == "**") {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E015",
                    message = "the path $pattern has the wildcard segment $segment, which the IR cannot represent",
                )
                PathSegment.Literal(segment)
            } else {
                PathSegment.Literal(segment)
            }
        }
    }

    private fun response(handler: HandlerMethod, info: RequestMappingInfo): List<IrResponse> {
        val returnType = ResolvableType.forMethodReturnType(handler.method)
        val raw = returnType.resolve()
        if (raw != null && (raw.name.startsWith("reactor.") ||
                raw.name == "org.springframework.web.servlet.mvc.method.annotation.SseEmitter" ||
                raw.name == "org.springframework.web.servlet.mvc.method.annotation.StreamingResponseBody")
        ) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E016",
                message = "the handler ${handler} returns a stream, which this slice does not lower",
            )
            return emptyList()
        }
        val bodyType = unwrap(returnType).resolve()
        val direct = raw != ResponseEntity::class.java && raw != HttpEntity::class.java
        val annotation = AnnotatedElementUtils.findMergedAnnotation(handler.method, ResponseStatus::class.java)
            ?: AnnotatedElementUtils.findMergedAnnotation(handler.beanType, ResponseStatus::class.java)
        val status = when {
            annotation != null -> IrResponseStatus.Exact(annotation.value.value())
            direct -> IrResponseStatus.Exact(200)
            else -> {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Warning,
                    code = "SPRING-W003",
                    message = "the handler ${handler} returns ResponseEntity without @ResponseStatus, so its status is recorded as unknown",
                )
                IrResponseStatus.Unknown
            }
        }
        val content = if (bodyType == ProblemDetail::class.java) {
            ContentKind.ProblemJson
        } else {
            responseContent(info)
        }
        val empty = bodyType == null ||
            bodyType == Void::class.java ||
            bodyType == java.lang.Void.TYPE ||
            bodyType == Unit::class.java
        val mediaType = when {
            empty -> null
            content == ContentKind.ProblemJson -> "application/problem+json"
            else -> info.producesCondition.producibleMediaTypes.firstOrNull()?.toString()
        }
        val body = when {
            empty || content == ContentKind.Empty -> null
            bodyType == ProblemDetail::class.java -> TypeRef.Inline(InlineType.AnyType)
            else -> types.ref(
                mapper.typeFactory.constructType(unwrap(returnType).type),
                Direction.Output,
            )
        }
        return listOf(
            IrResponse(
                status = status,
                content = if (empty) ContentKind.Empty else content,
                mediaType = if (empty) null else mediaType,
                body = body,
            ),
        )
    }

    private fun unwrap(type: ResolvableType): ResolvableType {
        val raw = type.resolve()
        return if (raw == ResponseEntity::class.java || raw == HttpEntity::class.java) {
            unwrap(type.getGeneric(0))
        } else {
            type
        }
    }

    private fun requestContent(info: RequestMappingInfo): RequestContent {
        val mediaTypes = info.consumesCondition.consumableMediaTypes.toList()
        val kind = when {
            mediaTypes.any { it.subtype == "multipart" } -> ContentKind.Multipart
            mediaTypes.any { it.subtype == "x-www-form-urlencoded" } -> ContentKind.Form
            mediaTypes.any { it.type == "text" } -> ContentKind.Text
            mediaTypes.any { it.subtype == "octet-stream" } -> ContentKind.Bytes
            else -> ContentKind.Json
        }
        return RequestContent(kind, mediaTypes.firstOrNull()?.toString())
    }

    private fun responseContent(info: RequestMappingInfo): ContentKind {
        val mediaTypes = info.producesCondition.producibleMediaTypes
        return when {
            mediaTypes.any { it.type == "text" } -> ContentKind.Text
            mediaTypes.any { it.subtype == "problem+json" } -> ContentKind.ProblemJson
            mediaTypes.any { it.subtype == "octet-stream" } -> ContentKind.Bytes
            else -> ContentKind.Json
        }
    }

    private fun parameterRef(
        parameter: org.springframework.core.MethodParameter,
        shape: KotlinShape,
        direction: Direction,
    ): TypeRef {
        val format = parameter.getParameterAnnotation(com.fasterxml.jackson.annotation.JsonFormat::class.java)
        val serialize = parameter.getParameterAnnotation(JsonSerialize::class.java)
        val hints = wireHints(
            format = format,
            stringSerializer = serialize?.using?.java == ToStringSerializer::class.java,
            nullable = shape.nullable,
        )
        return types.ref(
            mapper.typeFactory.constructType(parameter.genericParameterType),
            direction,
            hints,
        )
    }

    private fun parameterName(
        parameter: org.springframework.core.MethodParameter,
        explicit: String,
    ): String {
        if (explicit.isNotEmpty()) return explicit
        val discovered = parameter.method?.let { parameterNameDiscoverer.getParameterNames(it) }
        return parameter.parameterName
            ?: discovered?.getOrNull(parameter.parameterIndex)
            ?: "arg${parameter.parameterIndex}"
    }

    private fun stringType(): TypeRef = TypeRef.Inline(InlineType.StringType())

    private fun scalar(type: Class<*>): Boolean =
        type.isPrimitive ||
            type == String::class.java ||
            type == Boolean::class.javaPrimitiveType || type == Boolean::class.javaObjectType ||
            type == Char::class.javaPrimitiveType || type == Char::class.javaObjectType ||
            type == Byte::class.javaPrimitiveType || type == Byte::class.javaObjectType ||
            type == Short::class.javaPrimitiveType || type == Short::class.javaObjectType ||
            type == Int::class.javaPrimitiveType || type == Int::class.javaObjectType ||
            type == Long::class.javaPrimitiveType || type == Long::class.javaObjectType ||
            type == Float::class.javaPrimitiveType || type == Float::class.javaObjectType ||
            type == Double::class.javaPrimitiveType || type == Double::class.javaObjectType ||
            type == BigDecimal::class.java ||
            type == BigInteger::class.java ||
            type == UUID::class.java ||
            type == Instant::class.java ||
            type == OffsetDateTime::class.java ||
            type == ZonedDateTime::class.java ||
            type == LocalDateTime::class.java ||
            type == LocalDate::class.java ||
            type == LocalTime::class.java ||
            type == Duration::class.java ||
            type == Date::class.java ||
            type.isEnum

    private fun irMethod(requestMethod: RequestMethod): IrMethod? =
        IrMethod.entries.firstOrNull { it.name.equals(requestMethod.name, ignoreCase = true) }

    private fun ignored(handler: HandlerMethod): Boolean =
        AnnotatedElementUtils.hasAnnotation(handler.beanType, PonsIgnore::class.java) ||
            AnnotatedElementUtils.hasAnnotation(handler.method, PonsIgnore::class.java) ||
            handler.beanType.name in frameworkControllers

    private fun meta(handler: HandlerMethod): Meta {
        val annotation = AnnotatedElementUtils.findMergedAnnotation(
            handler.method,
            PonsOperation::class.java,
        ) ?: return Meta(null, null)
        val doc = if (annotation.summary.isNotEmpty() ||
            annotation.description.isNotEmpty() ||
            annotation.tags.isNotEmpty()
        ) {
            Doc(
                summary = annotation.summary.ifEmpty { null },
                description = annotation.description.ifEmpty { null },
                tags = annotation.tags.toList(),
            )
        } else {
            null
        }
        val provides = provision(annotation.provides, handler)
        return Meta(doc, provides)
    }

    private fun provision(provides: Provides, handler: HandlerMethod): CredentialProvision? {
        if (provides.scheme.isEmpty()) return null
        if (provides.from.isEmpty()) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E017",
                message = "the operation ${handler} provides ${provides.scheme} without a credential locator",
            )
            return null
        }
        val value = runCatching { parseLocator(provides.from) }.getOrNull()
        if (value == null) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E018",
                message = "the credential locator ${provides.from} on ${handler} is not supported",
            )
            return null
        }
        val expiresAt = provides.expiresAt.takeIf { it.isNotEmpty() }?.let { locator ->
            runCatching { parseLocator(locator) }.getOrNull() ?: run {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E019",
                    message = "the credential expiry locator $locator on ${handler} is not supported",
                )
                null
            }
        }
        return CredentialProvision(
            scheme = provides.scheme,
            value = value,
            expiresAt = expiresAt,
        )
    }

    private data class RequestContent(val kind: ContentKind, val mediaType: String?)

    private data class Meta(val doc: Doc?, val provides: CredentialProvision?)
}
