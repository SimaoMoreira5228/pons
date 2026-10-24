package com.pons.spring

import com.pons.ir.AuthScheme
import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Document
import com.pons.ir.ErrorStatus
import com.pons.ir.Info
import com.pons.ir.Operation
import org.springframework.context.ApplicationContext
import org.springframework.web.servlet.mvc.method.annotation.RequestMappingHandlerMapping

internal class PonsDocumentBuilder(
    private val context: ApplicationContext,
    private val mapper: com.fasterxml.jackson.databind.ObjectMapper,
    private val info: Info,
    private val defaultAuthScheme: String?,
) {
    fun build(): Document {
        val diagnostics = mutableListOf<Diagnostic>()
        val types = JacksonTypes(mapper, diagnostics)
        val schemes = authSchemes(diagnostics)
        val auth = AuthIntrospector(defaultAuthScheme, diagnostics)
        val errors = ErrorIntrospector(types, diagnostics)
        val routes = RouteIntrospector(mapper, types, auth, errors, diagnostics)
        val mapping = context.getBean(RequestMappingHandlerMapping::class.java)
        val operations = routes.lower(mapping)
        val commonErrors = errors.commonErrors(context)
        ensureSchemes(operations, schemes, diagnostics)
        val settled = operations.map { operation ->
            val kept = operation.errors.filter { error ->
                commonErrors.none { common -> shadows(common.status, error.status) }
            }
            if (kept.size != operation.errors.size) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Warning,
                    code = "SPRING-W004",
                    message = "the operation ${operation.id} declares an error that the common errors already answer",
                )
                operation.copy(errors = kept)
            } else {
                operation
            }
        }
        return Document(
            irVersion = "0.2",
            info = info,
            types = types.buildAll(),
            authSchemes = schemes.toSortedMap(),
            commonErrors = commonErrors.sortedWith(compareBy { statusOrder(it.status) }),
            operations = settled.sortedWith { left, right -> compareIds(left.id, right.id) },
            diagnostics = diagnostics.toList(),
        )
    }

    private fun authSchemes(diagnostics: MutableList<Diagnostic>): MutableMap<String, AuthScheme> {
        val result = linkedMapOf<String, AuthScheme>()
        for (name in context.beanDefinitionNames) {
            val beanType = context.getType(name) ?: continue
            val annotations = beanType.getAnnotationsByType(PonsAuthScheme::class.java).toList()
                .ifEmpty {
                    beanType.getAnnotation(PonsAuthSchemes::class.java)?.value?.toList().orEmpty()
                }
            for (annotation in annotations) {
                val scheme = authScheme(annotation, diagnostics) ?: continue
                val previous = result.put(annotation.id, scheme)
                if (previous != null && previous != scheme) {
                    diagnostics += Diagnostic(
                        severity = DiagnosticSeverity.Error,
                        code = "SPRING-E020",
                        message = "the auth scheme ${annotation.id} is declared more than once with different definitions",
                    )
                }
            }
        }
        return result
    }

    private fun ensureSchemes(
        operations: List<Operation>,
        schemes: MutableMap<String, AuthScheme>,
        diagnostics: MutableList<Diagnostic>,
    ) {
        val referenced = operations.flatMap { operation ->
            operation.auth.alternatives.flatten() + listOfNotNull(operation.provides?.scheme)
        }.toSet()
        for (id in referenced) {
            if (schemes.containsKey(id)) continue
            schemes[id] = AuthScheme.CustomScheme(description = id)
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Warning,
                code = "SPRING-W005",
                message = "the auth scheme $id is referenced but not declared; it is recorded as a custom scheme",
            )
        }
    }

    private fun shadows(common: ErrorStatus, operation: ErrorStatus): Boolean = when {
        common == ErrorStatus.Default && operation == ErrorStatus.Default -> true
        common is ErrorStatus.ClassStatus && operation is ErrorStatus.ClassStatus ->
            common.code == operation.code
        else -> false
    }

    private fun statusOrder(status: ErrorStatus): Int = when (status) {
        is ErrorStatus.Exact -> status.code
        is ErrorStatus.ClassStatus -> status.code * 100
        ErrorStatus.Default -> 999
    }

    private fun compareIds(left: List<List<String>>, right: List<List<String>>): Int {
        val leftWords = left.flatten()
        val rightWords = right.flatten()
        for (index in 0 until minOf(leftWords.size, rightWords.size)) {
            val comparison = leftWords[index].compareTo(rightWords[index])
            if (comparison != 0) return comparison
        }
        return leftWords.size.compareTo(rightWords.size)
    }
}
