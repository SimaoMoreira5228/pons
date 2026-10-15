package com.pons.spring

import com.fasterxml.jackson.databind.ObjectMapper
import com.pons.ir.Document
import com.pons.ir.Method
import com.pons.ir.Violation
import jakarta.servlet.FilterChain
import jakarta.servlet.ReadListener
import jakarta.servlet.ServletInputStream
import jakarta.servlet.http.HttpServletRequest
import jakarta.servlet.http.HttpServletRequestWrapper
import jakarta.servlet.http.HttpServletResponse
import org.apache.commons.logging.LogFactory
import org.springframework.http.MediaType
import org.springframework.web.filter.OncePerRequestFilter
import java.io.BufferedReader
import java.io.ByteArrayInputStream
import java.io.InputStreamReader
import java.nio.charset.Charset

class PonsRequestValidationFilter(
    private val exporter: PonsExporter,
    private val mapper: ObjectMapper,
) : OncePerRequestFilter() {
    private val validator = PonsRequestValidator(mapper)
    private val document: Document? by lazy {
        runCatching { exporter.document() }.getOrElse { error ->
            log.warn(
                "pons request validation is skipped because the document cannot be built",
                error,
            )
            null
        }
    }

    override fun doFilterInternal(
        request: HttpServletRequest,
        response: HttpServletResponse,
        chain: FilterChain,
    ) {
        val resolved = document ?: return chain.doFilter(request, response)
        val path = applicationPath(request)
        val operation = validator.operation(resolved, request.method, path)
        if (operation == null || !validator.rejects(operation)) {
            return chain.doFilter(request, response)
        }
        val body = if (validator.readsBody(operation, request)) {
            try {
                request.inputStream.readAllBytes()
            } catch (error: Exception) {
                return reject(
                    operation,
                    listOf(Violation("", "expected a readable request body, found ${error.message}")),
                    response,
                )
            }
        } else {
            null
        }
        val content = validator.contentViolation(operation, request, body)
        val violations = if (content != null) {
            listOf(content)
        } else {
            validator.validate(resolved, operation, request, path, body)
        }
        if (violations.isNotEmpty()) {
            return reject(operation, violations, response)
        }
        chain.doFilter(if (body == null) request else CachedBodyRequest(request, body), response)
    }

    private fun reject(
        operation: com.pons.ir.Operation,
        violations: List<Violation>,
        response: HttpServletResponse,
    ) {
        response.status = REJECTED_STATUS
        if (operation.method == Method.Head) return
        response.contentType = MediaType.APPLICATION_JSON_VALUE
        val report = mapper.createObjectNode()
        val array = report.putArray("violations")
        for (violation in violations) {
            array.addObject()
                .put("path", violation.path)
                .put("reason", violation.reason)
        }
        mapper.writeValue(response.outputStream, report)
    }

    private fun applicationPath(request: HttpServletRequest): String {
        val uri = request.requestURI
        val context = request.contextPath
        val relative = if (context.isNotEmpty() && uri.startsWith(context)) {
            uri.substring(context.length)
        } else {
            uri
        }
        return relative.ifEmpty { "/" }
    }
}

private val log = LogFactory.getLog(PonsRequestValidationFilter::class.java)

private class CachedBodyRequest(
    request: HttpServletRequest,
    private val body: ByteArray,
) : HttpServletRequestWrapper(request) {
    private val charset: Charset =
        request.characterEncoding?.let { runCatching { Charset.forName(it) }.getOrNull() }
            ?: Charsets.UTF_8
    private val parameters: Map<String, List<String>>? =
        if (isForm(request)) collectParameters(request) else null

    override fun getInputStream(): ServletInputStream {
        val stream = ByteArrayInputStream(body)
        return object : ServletInputStream() {
            override fun read(): Int = stream.read()

            override fun read(buffer: ByteArray, offset: Int, length: Int): Int =
                stream.read(buffer, offset, length)

            override fun isFinished(): Boolean = stream.available() == 0

            override fun isReady(): Boolean = true

            override fun setReadListener(listener: ReadListener?) = Unit
        }
    }

    override fun getReader(): BufferedReader = BufferedReader(InputStreamReader(getInputStream(), charset))

    override fun getContentLength(): Int = body.size

    override fun getContentLengthLong(): Long = body.size.toLong()

    override fun getParameter(name: String): String? =
        parameters?.get(name)?.firstOrNull() ?: super.getParameter(name)

    override fun getParameterValues(name: String): Array<String>? =
        parameters?.get(name)?.toTypedArray() ?: super.getParameterValues(name)

    override fun getParameterNames(): java.util.Enumeration<String> {
        val source = parameters ?: return super.getParameterNames()
        val names = source.keys.toList()
        return object : java.util.Enumeration<String> {
            private val iterator = names.iterator()

            override fun hasMoreElements(): Boolean = iterator.hasNext()

            override fun nextElement(): String = iterator.next()
        }
    }

    override fun getParameterMap(): Map<String, Array<String>> =
        parameters?.mapValues { it.value.toTypedArray() } ?: super.getParameterMap()

    private fun collectParameters(request: HttpServletRequest): Map<String, List<String>> {
        val groups = LinkedHashMap<String, MutableList<String>>()
        fun add(pairs: List<Pair<String, String>>) {
            for ((name, value) in pairs) {
                groups.getOrPut(name) { mutableListOf() }.add(value)
            }
        }
        add(decodePairs(request.queryString.orEmpty()))
        if (isForm(request)) add(decodePairs(body.toString(charset)))
        return groups
    }

    private fun isForm(request: HttpServletRequest): Boolean =
        request.contentType?.substringBefore(';')?.trim()
            ?.equals("application/x-www-form-urlencoded", ignoreCase = true) == true
}
