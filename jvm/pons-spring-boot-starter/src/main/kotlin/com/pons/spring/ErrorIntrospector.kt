package com.pons.spring

import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Direction
import com.pons.ir.ErrorContent
import com.pons.ir.ErrorResponse
import com.pons.ir.ErrorStatus
import com.pons.ir.InlineType
import com.pons.ir.Presence
import com.pons.ir.ResponseHeader
import com.pons.ir.TypeRef
import org.springframework.aop.support.AopUtils
import org.springframework.context.ApplicationContext
import org.springframework.core.ResolvableType
import org.springframework.core.annotation.AnnotatedElementUtils
import org.springframework.http.HttpEntity
import org.springframework.http.ProblemDetail
import org.springframework.http.ResponseEntity
import org.springframework.util.ReflectionUtils
import org.springframework.web.bind.annotation.ExceptionHandler
import org.springframework.web.bind.annotation.ResponseStatus
import org.springframework.web.bind.annotation.RestControllerAdvice
import org.springframework.web.method.HandlerMethod
import java.lang.reflect.Method

internal class ErrorIntrospector(
    private val types: JacksonTypes,
    private val diagnostics: MutableList<Diagnostic>,
) {
    fun handlerErrors(handler: HandlerMethod): List<ErrorResponse> {
        val declared = listOfNotNull(
            AnnotatedElementUtils.findMergedAnnotation(handler.method, PonsErrors::class.java),
            AnnotatedElementUtils.findMergedAnnotation(handler.beanType, PonsErrors::class.java),
        )
        return declared
            .flatMap { it.value.toList() }
            .distinctBy { it.java }
            .mapNotNull { errorOf(it.java) }
    }

    fun commonErrors(context: ApplicationContext): List<ErrorResponse> {
        val result = mutableListOf<ErrorResponse>()
        val slots = mutableSetOf<ErrorStatus>()
        for (bean in context.getBeansWithAnnotation(RestControllerAdvice::class.java).values) {
            val beanType = AopUtils.getTargetClass(bean)
            val advice = AnnotatedElementUtils.findMergedAnnotation(
                beanType,
                RestControllerAdvice::class.java,
            ) ?: continue
            if (!isGlobal(advice)) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Warning,
                    code = "SPRING-W010",
                    message = "the advice ${beanType.name} is restricted to some controllers, so it is not recorded as a common error",
                )
                continue
            }
            for (method in ReflectionUtils.getAllDeclaredMethods(beanType)) {
                val error = commonError(beanType, method) ?: continue
                if (!slots.add(error.status)) {
                    diagnostics += Diagnostic(
                        severity = DiagnosticSeverity.Warning,
                        code = "SPRING-W011",
                        message = "more than one exception handler answers ${error.status}; only ${method.name} is recorded",
                    )
                    continue
                }
                result += error
            }
        }
        return result
    }

    private fun isGlobal(advice: RestControllerAdvice): Boolean =
        advice.basePackages.isEmpty() &&
            advice.basePackageClasses.isEmpty() &&
            advice.assignableTypes.isEmpty() &&
            advice.annotations.isEmpty()

    private fun commonError(beanType: Class<*>, method: Method): ErrorResponse? {
        val handler = AnnotatedElementUtils.findMergedAnnotation(method, ExceptionHandler::class.java)
            ?: return null
        val exceptionType = handler.value.firstOrNull()?.java
        val status = errorStatus(beanType, method, exceptionType)
        if (status == null) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E015",
                message = "the exception handler ${beanType.name}.${method.name} returns a status at runtime, which this slice cannot infer",
            )
            return null
        }
        val body = unwrap(ResolvableType.forMethodReturnType(method))
        val bodyType = body.resolve()
        val content = when {
            bodyType == null || bodyType == Void::class.java || bodyType == Unit::class.java ->
                ErrorContent.Empty
            bodyType == ProblemDetail::class.java -> ErrorContent.ProblemJson
            else -> ErrorContent.Json
        }
        val bodyRef = when {
            content == ErrorContent.Empty -> null
            bodyType == ProblemDetail::class.java -> TypeRef.Inline(InlineType.AnyType)
            bodyType != null -> types.named(bodyType, Direction.Output)
            else -> null
        }
        val name = when {
            bodyType != null && bodyType != Void::class.java && bodyType != Unit::class.java ->
                typeName(bodyType)
            exceptionType != null -> typeName(exceptionType)
            else -> words(method.name)
        }
        return ErrorResponse(
            status = ErrorStatus.Exact(status),
            name = name,
            content = content,
            body = bodyRef,
        )
    }

    private fun errorStatus(beanType: Class<*>, method: Method, exceptionType: Class<*>?): Int? {
        val annotation = AnnotatedElementUtils.findMergedAnnotation(method, ResponseStatus::class.java)
            ?: AnnotatedElementUtils.findMergedAnnotation(beanType, ResponseStatus::class.java)
        annotation?.let { return it.value.value() }
        exceptionType?.getAnnotation(PonsError::class.java)?.let { return it.status }
        val raw = ResolvableType.forMethodReturnType(method).resolve()
        if (raw == ResponseEntity::class.java || raw == HttpEntity::class.java) return null
        return 500
    }

    private fun errorOf(beanType: Class<*>): ErrorResponse? {
        val annotation = AnnotatedElementUtils.findMergedAnnotation(beanType, PonsError::class.java)
        if (annotation == null) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E016",
                message = "${beanType.name} is listed in @PonsErrors but has no @PonsError",
            )
            return null
        }
        if (annotation.status !in 100..599) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E017",
                message = "${beanType.name} declares the invalid error status ${annotation.status}",
            )
            return null
        }
        val content = when (annotation.content) {
            PonsErrorContent.JSON -> ErrorContent.Json
            PonsErrorContent.PROBLEM_JSON -> ErrorContent.ProblemJson
            PonsErrorContent.TEXT -> ErrorContent.Text
            PonsErrorContent.EMPTY -> ErrorContent.Empty
        }
        return ErrorResponse(
            status = ErrorStatus.Exact(annotation.status),
            name = typeName(beanType),
            content = content,
            body = if (content == ErrorContent.Empty) null else types.named(beanType, Direction.Output),
            headers = annotation.headers.map { header ->
                ResponseHeader(
                    wire = header.name,
                    ty = TypeRef.Inline(InlineType.StringType()),
                    presence = if (header.required) Presence.Required else Presence.Optional,
                )
            },
            retryable = annotation.retryable,
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
}
