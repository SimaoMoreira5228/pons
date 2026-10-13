package com.pons.spring

import com.pons.ir.CredentialLocator
import java.lang.reflect.Method

private val wordPattern = Regex("([A-Z]+(?=[A-Z][a-z]|\\d|$))|([A-Z]?[a-z]+)|(\\d+)")

internal fun words(value: String): List<String> {
    val parsed = wordPattern.findAll(value)
        .map { it.value.lowercase() }
        .filter { it.isNotEmpty() }
        .toList()
    return parsed.ifEmpty { listOf(value.lowercase().ifEmpty { "value" }) }
}

internal fun operationId(value: String): List<List<String>> {
    val segments = value.split('.')
        .filter { it.isNotEmpty() }
        .map { segment ->
            segment.split('_')
                .filter { it.isNotEmpty() }
                .flatMap(::words)
                .ifEmpty { listOf(segment) }
        }
    require(segments.isNotEmpty()) { "operation id $value has no segments" }
    return segments
}

internal fun inferredOperationId(beanType: Class<*>, method: Method): List<List<String>> {
    val controller = words(beanType.simpleName)
        .let { names -> if (names.lastOrNull() == "controller") names.dropLast(1) else names }
        .ifEmpty { listOf("controller") }
    return listOf(controller, words(method.name))
}

internal fun typeName(beanType: Class<*>): List<String> {
    val simple = beanType.simpleName.ifEmpty { beanType.name.substringAfterLast('.') }
    return words(simple)
}

internal fun generatedTypeId(beanType: Class<*>, method: Method, suffix: String): String {
    val parameters = method.parameterTypes.joinToString(",") { it.simpleName.ifEmpty { it.name } }
    return "${beanType.name}#${method.name}($parameters)#$suffix"
}

internal fun parseLocator(value: String): CredentialLocator? {
    val separator = value.indexOf(':')
    require(separator > 0) { "credential locator $value needs a source and a value" }
    val source = value.substring(0, separator)
    val target = value.substring(separator + 1)
    require(target.isNotEmpty()) { "credential locator $value has an empty target" }
    return when (source) {
        "body" -> {
            require(target.startsWith("/")) { "body credential locator $value needs a JSON pointer" }
            CredentialLocator.BodyLocator(target)
        }
        "header" -> CredentialLocator.HeaderLocator(target)
        "cookie" -> CredentialLocator.SetCookieLocator(target)
        else -> null
    }
}
