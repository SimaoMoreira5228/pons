package com.pons.spring

import com.pons.ir.Body
import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Direction
import com.pons.ir.Doc
import com.pons.ir.ErrorContent
import com.pons.ir.ErrorResponse
import com.pons.ir.ErrorStatus
import com.pons.ir.Field
import com.pons.ir.InlineType
import com.pons.ir.Method
import com.pons.ir.NamedRef
import com.pons.ir.ObjectOpenness
import com.pons.ir.Presence
import com.pons.ir.Response
import com.pons.ir.ResponseStatus
import com.pons.ir.TypeDef
import com.pons.ir.TypeKind
import com.pons.ir.TypeRef

internal const val REJECTED_STATUS = 422
internal val REJECTED_NAME = listOf("request", "rejected")
internal const val REPORT_TYPE_ID = "com.pons.spring.ViolationReport#output"
internal const val VIOLATION_TYPE_ID = "com.pons.spring.Violation#output"

private const val REJECTION_SUMMARY = "The request does not satisfy the published request contract."
private const val VIOLATION_ORIGIN = "com.pons.spring.RequestRejection#Violation"
private const val REPORT_ORIGIN = "com.pons.spring.RequestRejection#ViolationReport"

internal fun requestRejection(
    method: Method,
    errors: List<ErrorResponse>,
    diagnostics: MutableList<Diagnostic>,
): ErrorResponse? {
    val declared = errors.firstOrNull { it.status == ErrorStatus.Exact(REJECTED_STATUS) }
    if (declared != null) {
        if (method == Method.Head && declared.body != null) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E023",
                message = "a HEAD operation cannot serve the request rejection as ${declared.name}, which carries a body",
            )
            return null
        }
        diagnostics += Diagnostic(
            severity = DiagnosticSeverity.Warning,
            code = "SPRING-W012",
            message = "a request is rejected as ${declared.name}, the 422 the operation already declares",
        )
        return null
    }
    val body = when (method) {
        Method.Head -> null
        else -> TypeRef.Named(NamedRef(REPORT_TYPE_ID))
    }
    return ErrorResponse(
        status = ErrorStatus.Exact(REJECTED_STATUS),
        name = REJECTED_NAME,
        content = if (body == null) ErrorContent.Empty else ErrorContent.Json,
        body = body,
        retryable = false,
        doc = Doc(summary = REJECTION_SUMMARY),
    )
}

internal fun JacksonTypes.publishViolationReport() {
    define(
        VIOLATION_TYPE_ID,
        TypeDef(
            name = listOf("violation"),
            direction = Direction.Output,
            definition = TypeKind.StructType(
                fields = listOf(
                    Field(
                        wire = "path",
                        ident = listOf("path"),
                        ty = stringType(),
                        presence = Presence.Required,
                    ),
                    Field(
                        wire = "reason",
                        ident = listOf("reason"),
                        ty = stringType(),
                        presence = Presence.Required,
                    ),
                ),
                openness = ObjectOpenness.Reject,
            ),
            origin = VIOLATION_ORIGIN,
        ),
    )
    define(
        REPORT_TYPE_ID,
        TypeDef(
            name = listOf("violation", "report"),
            direction = Direction.Output,
            definition = TypeKind.StructType(
                fields = listOf(
                    Field(
                        wire = "violations",
                        ident = listOf("violations"),
                        ty = TypeRef.Inline(
                            InlineType.ArrayType(
                                item = TypeRef.Named(NamedRef(VIOLATION_TYPE_ID)),
                            ),
                        ),
                        presence = Presence.Required,
                    ),
                ),
                openness = ObjectOpenness.Reject,
            ),
            origin = REPORT_ORIGIN,
        ),
    )
}

internal fun validatedBody(kind: com.pons.ir.ContentKind, ty: TypeRef): Boolean = when (kind) {
    com.pons.ir.ContentKind.Json -> true
    com.pons.ir.ContentKind.Form -> ty is TypeRef.Named
    else -> false
}

internal fun declareFrameworkRejections(
    body: Body,
    responses: List<Response>,
    errors: List<ErrorResponse>,
    diagnostics: MutableList<Diagnostic>,
) {
    if (!frameworkReadsTheBody(body)) return
    val undeclared = listOf(400, 413).filter { status ->
        errors.none { it.status.covers(status) } &&
            responses.none { it.status.covers(status) }
    }
    if (undeclared.isEmpty()) return
    diagnostics += Diagnostic(
        severity = DiagnosticSeverity.Warning,
        code = "SPRING-W013",
        message = "the framework reads this request body itself, so it answers a body it cannot read as 400 and a body over the limit as 413 before the handler runs; the operation declares none of ${undeclared.joinToString(" or ")}",
    )
}

private fun frameworkReadsTheBody(body: Body): Boolean = when (body.content) {
    com.pons.ir.ContentKind.Multipart,
    com.pons.ir.ContentKind.Text,
    com.pons.ir.ContentKind.Bytes,
    -> true
    com.pons.ir.ContentKind.Form -> body.ty !is TypeRef.Named
    com.pons.ir.ContentKind.Json,
    com.pons.ir.ContentKind.Stream,
    com.pons.ir.ContentKind.ProblemJson,
    com.pons.ir.ContentKind.Empty,
    -> false
}

private fun ErrorStatus.covers(status: Int): Boolean = when (this) {
    is ErrorStatus.Exact -> code == status
    is ErrorStatus.ClassStatus -> status / 100 == code
    ErrorStatus.Default -> true
}

private fun ResponseStatus.covers(status: Int): Boolean = when (this) {
    is ResponseStatus.Exact -> code == status
    is ResponseStatus.ClassStatus -> status / 100 == code
    ResponseStatus.Default, ResponseStatus.Unknown -> false
}

private fun stringType(): TypeRef = TypeRef.Inline(InlineType.StringType())
