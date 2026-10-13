package com.pons.spring

import kotlin.reflect.KClass
import kotlin.reflect.KFunction
import kotlin.reflect.KParameter
import kotlin.reflect.full.memberProperties
import kotlin.reflect.full.primaryConstructor
import kotlin.reflect.jvm.kotlinFunction
import java.lang.reflect.Method

internal data class KotlinShape(
    val nullable: Boolean = false,
    val optional: Boolean = false,
)

internal class KotlinTypeInfo(beanType: Class<*>) {
    val hasMetadata: Boolean = beanType.getAnnotation(Metadata::class.java) != null
    private val kClass: KClass<*>? = runCatching { beanType.kotlin }.getOrNull()
    private val constructorParameters: Map<String, KParameter> =
        runCatching {
            kClass?.primaryConstructor?.parameters
                ?.filter { it.kind == KParameter.Kind.VALUE }
                ?.mapNotNull { parameter -> parameter.name?.let { it to parameter } }
                ?.toMap()
                .orEmpty()
        }.getOrDefault(emptyMap())
    private val properties = runCatching { kClass?.memberProperties.orEmpty().associateBy { it.name } }
        .getOrDefault(emptyMap())

    fun property(internalName: String): KotlinShape {
        val parameter = constructorParameters[internalName]
        val property = properties[internalName]
        val nullable = parameter?.type?.isMarkedNullable == true ||
            property?.returnType?.isMarkedNullable == true
        return KotlinShape(nullable = nullable, optional = parameter?.isOptional == true)
    }
}

internal fun kotlinMethodShape(method: Method, index: Int): KotlinShape {
    val function: KFunction<*>? = runCatching { method.kotlinFunction }.getOrNull()
    val parameter = function?.parameters
        ?.filter { it.kind == KParameter.Kind.VALUE }
        ?.getOrNull(index)
        ?: return KotlinShape()
    return KotlinShape(
        nullable = parameter.type.isMarkedNullable,
        optional = parameter.isOptional,
    )
}
