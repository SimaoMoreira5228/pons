package com.pons.spring

import com.fasterxml.jackson.annotation.JsonFormat
import com.fasterxml.jackson.annotation.JsonIgnoreProperties
import com.fasterxml.jackson.annotation.JsonInclude
import com.fasterxml.jackson.annotation.JsonProperty
import com.fasterxml.jackson.annotation.JsonPropertyDescription
import com.fasterxml.jackson.annotation.JsonSubTypes
import com.fasterxml.jackson.annotation.JsonTypeInfo
import com.fasterxml.jackson.annotation.JsonTypeName
import com.fasterxml.jackson.annotation.JsonValue
import com.fasterxml.jackson.annotation.Nulls
import com.fasterxml.jackson.databind.DeserializationFeature
import com.fasterxml.jackson.databind.JavaType
import com.fasterxml.jackson.databind.ObjectMapper
import com.fasterxml.jackson.databind.SerializationFeature
import com.fasterxml.jackson.databind.introspect.AnnotatedClass
import com.fasterxml.jackson.databind.introspect.AnnotatedMember
import com.fasterxml.jackson.databind.introspect.BeanPropertyDefinition
import com.fasterxml.jackson.databind.ser.std.ToStringSerializer
import com.pons.ir.Access
import com.pons.ir.ByteEncoding
import com.pons.ir.DecimalWire
import com.pons.ir.Diagnostic
import com.pons.ir.DiagnosticSeverity
import com.pons.ir.Direction
import com.pons.ir.Doc
import com.pons.ir.EnumRepr
import com.pons.ir.Field as IrField
import com.pons.ir.FieldDefault
import com.pons.ir.FieldFormat
import com.pons.ir.InlineType
import com.pons.ir.IntegerWire
import com.pons.ir.NamedRef
import com.pons.ir.ObjectOpenness
import com.pons.ir.Presence
import com.pons.ir.TypeDef
import com.pons.ir.TypeKind
import com.pons.ir.TypeRef
import com.pons.ir.Variant
import com.pons.ir.VariantPayload
import kotlinx.serialization.json.JsonPrimitive
import java.lang.reflect.Field as JavaField
import java.lang.reflect.Method as JavaMethod
import java.math.BigDecimal
import java.math.BigInteger
import java.net.URI
import java.net.URL
import java.time.Duration
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.ZonedDateTime
import java.util.Date
import java.util.Optional
import java.util.UUID

internal data class WireHints(
    val nullable: Boolean = false,
    val shape: JsonFormat.Shape = JsonFormat.Shape.ANY,
    val stringWire: Boolean = false,
)

internal fun wireHints(
    format: JsonFormat?,
    stringSerializer: Boolean,
    nullable: Boolean,
): WireHints = WireHints(
    nullable = nullable,
    shape = format?.shape ?: JsonFormat.Shape.ANY,
    stringWire = stringSerializer,
)

internal fun wireHints(member: AnnotatedMember?, nullable: Boolean): WireHints = wireHints(
    format = member?.getAnnotation(JsonFormat::class.java),
    stringSerializer = member.isStringSerializer(),
    nullable = nullable,
)

internal fun wireHints(element: JavaMethod?, nullable: Boolean): WireHints = wireHints(
    format = element?.getAnnotation(JsonFormat::class.java),
    stringSerializer = element.isStringSerializer(),
    nullable = nullable,
)

internal fun wireHints(element: JavaField?, nullable: Boolean): WireHints = wireHints(
    format = element?.getAnnotation(JsonFormat::class.java),
    stringSerializer = element.isStringSerializer(),
    nullable = nullable,
)

private fun AnnotatedMember?.isStringSerializer(): Boolean =
    this?.getAnnotation(com.fasterxml.jackson.databind.annotation.JsonSerialize::class.java)
        ?.using?.java == ToStringSerializer::class.java

private fun JavaMethod?.isStringSerializer(): Boolean =
    this?.getAnnotation(com.fasterxml.jackson.databind.annotation.JsonSerialize::class.java)
        ?.using?.java == ToStringSerializer::class.java

private fun JavaField?.isStringSerializer(): Boolean =
    this?.getAnnotation(com.fasterxml.jackson.databind.annotation.JsonSerialize::class.java)
        ?.using?.java == ToStringSerializer::class.java

internal class JacksonTypes(
    private val mapper: ObjectMapper,
    private val diagnostics: MutableList<Diagnostic>,
) {
    private val classes = LinkedHashMap<String, Class<*>>()
    private val wanted = LinkedHashMap<String, MutableSet<Direction>>()
    private val built = LinkedHashMap<String, TypeDef>()
    private val fixed = LinkedHashMap<String, TypeDef>()
    private val annotatedClasses = HashMap<String, AnnotatedClass>()
    private val typeInfoCache = HashMap<String, JsonTypeInfo?>()

    fun named(beanType: Class<*>, direction: Direction, nullable: Boolean = false): TypeRef {
        val id = beanType.name
        classes[id] = beanType
        wanted.getOrPut(id) { mutableSetOf() }.add(direction)
        return nullable(TypeRef.Named(NamedRef(id)), nullable)
    }

    fun define(id: String, definition: TypeDef) {
        fixed[id] = definition
    }

    fun ref(type: JavaType, direction: Direction, hints: WireHints = WireHints()): TypeRef {
        val raw = type.rawClass
        val inline = when {
            raw == java.lang.Void.TYPE || raw == Void::class.java || raw == Unit::class.java ->
                InlineType.NullType
            raw == Any::class.java || raw == Any::class.javaObjectType ->
                InlineType.AnyType
            raw == String::class.java || raw == CharSequence::class.java ||
                raw == Char::class.java || raw == Char::class.javaObjectType ->
                InlineType.StringType()
            raw == Boolean::class.javaPrimitiveType || raw == Boolean::class.javaObjectType ->
                InlineType.BoolType
            raw == Byte::class.javaPrimitiveType || raw == Byte::class.javaObjectType ->
                intType(8, true, hints)
            raw == Short::class.javaPrimitiveType || raw == Short::class.javaObjectType ->
                intType(16, true, hints)
            raw == Int::class.javaPrimitiveType || raw == Int::class.javaObjectType ->
                intType(32, true, hints)
            raw == Long::class.javaPrimitiveType || raw == Long::class.javaObjectType ->
                intType(64, true, hints)
            raw == BigInteger::class.java ->
                intType(128, true, hints)
            raw == Float::class.javaPrimitiveType || raw == Float::class.javaObjectType ->
                InlineType.FloatType(32)
            raw == Double::class.javaPrimitiveType || raw == Double::class.javaObjectType ->
                InlineType.FloatType(64)
            raw == BigDecimal::class.java ->
                InlineType.DecimalType(decimalWire(hints))
            raw == UUID::class.java ->
                InlineType.StringType(format = FieldFormat.Uuid)
            raw == URI::class.java || raw == URL::class.java ->
                InlineType.StringType(format = FieldFormat.Uri)
            isTemporal(raw) -> temporalType(raw, hints)
            type.isArrayType && type.contentType?.rawClass == Byte::class.javaPrimitiveType ->
                InlineType.BytesType(ByteEncoding.Base64)
            type.isArrayType ->
                InlineType.ArrayType(ref(type.contentType, direction))
            raw == Optional::class.java ->
                InlineType.NullableType(ref(type.containedTypeOrUnknown(0), direction))
            raw == Set::class.java || Set::class.java.isAssignableFrom(raw) ->
                InlineType.ArrayType(ref(type.containedTypeOrUnknown(0), direction), unique = true)
            raw == Collection::class.java || Collection::class.java.isAssignableFrom(raw) ||
                Iterable::class.java.isAssignableFrom(raw) ->
                InlineType.ArrayType(ref(type.containedTypeOrUnknown(0), direction))
            raw == Map::class.java || Map::class.java.isAssignableFrom(raw) ->
                InlineType.MapType(
                    key = ref(type.containedTypeOrUnknown(0), direction),
                    value = ref(type.containedTypeOrUnknown(1), direction),
                )
            else -> null
        }
        val resolved = if (inline != null) TypeRef.Inline(inline) else named(raw, direction)
        return nullable(resolved, hints.nullable)
    }

    fun buildAll(): Map<String, TypeDef> {
        var changed: Boolean
        do {
            changed = false
            val snapshot = wanted.entries.map { it.key to it.value.toSet() }
            for ((id, directions) in snapshot) {
                val direction = combine(directions)
                if (built[id]?.direction != direction) {
                    built[id] = buildClass(id, classes.getValue(id), direction)
                    changed = true
                }
            }
        } while (changed)
        return (fixed + built).toSortedMap()
    }

    private fun buildClass(id: String, beanType: Class<*>, direction: Direction): TypeDef {
        if (beanType.isEnum) {
            return enumType(id, beanType, direction)
        }
        if (typeInfo(beanType) != null || isSealed(beanType)) {
            return unionType(id, beanType, direction)
        }
        valueSource(beanType)?.let { source ->
            return TypeDef(
                name = typeName(beanType),
                direction = direction,
                definition = TypeKind.NewtypeType(ref(source.type, direction, source.hints)),
                origin = id,
            )
        }
        if (classStringSerializer(beanType)) {
            return TypeDef(
                name = typeName(beanType),
                direction = direction,
                definition = TypeKind.NewtypeType(TypeRef.Inline(InlineType.StringType())),
                origin = id,
            )
        }
        return TypeDef(
            name = typeName(beanType),
            direction = direction,
            definition = TypeKind.StructType(
                fields = fields(beanType, direction),
                openness = openness(beanType, direction),
            ),
            origin = id,
        )
    }

    private fun fields(beanType: Class<*>, direction: Direction): List<IrField> {
        val output = if (direction != Direction.Input) {
            mapper.serializationConfig.introspect(mapper.typeFactory.constructType(beanType))
        } else {
            null
        }
        val input = if (direction != Direction.Output) {
            mapper.deserializationConfig.introspect(mapper.typeFactory.constructType(beanType))
        } else {
            null
        }
        val properties = LinkedHashMap<String, PropertyView>()
        output?.findProperties()
            ?.filter { it.couldSerialize() }
            ?.forEach { property ->
                val existing = properties[property.name]
                properties[property.name] = existing?.copy(outputProperty = property, output = true)
                    ?: PropertyView(outputProperty = property, output = true)
            }
        input?.findProperties()
            ?.filter { it.couldDeserialize() }
            ?.forEach { property ->
                val existing = properties[property.name]
                properties[property.name] = existing?.copy(inputProperty = property, input = true)
                    ?: PropertyView(inputProperty = property, input = true)
            }
        val shape = KotlinTypeInfo(beanType)
        val skipsNull = skipsNull(beanType)
        return properties.values.mapNotNull { view ->
            field(shape, skipsNull, view)
        }.sortedBy { it.wire }
    }

    private fun field(
        shape: KotlinTypeInfo,
        skipsNull: Boolean,
        view: PropertyView,
    ): IrField? {
        val outputProperty = view.outputProperty
        val inputProperty = view.inputProperty
        val property = outputProperty ?: inputProperty ?: return null
        val kotlin = shape.property(property.internalName)
        val members = members(property)
        val nullable = kotlin.nullable
        val input = view.input
        val output = view.output
        val required = outputProperty?.metadata?.required == true ||
            inputProperty?.metadata?.required == true
        val hasDefault = !inputProperty?.metadata?.defaultValue.isNullOrEmpty() ||
            !outputProperty?.metadata?.defaultValue.isNullOrEmpty()
        val inputOptional = input && !required &&
            (kotlin.optional || nullable || hasDefault || !shape.hasMetadata)
        val outputOptional = output &&
            (hasDefault || (nullable && skipsNull) ||
                outputProperty?.metadata?.valueNulls == Nulls.SKIP)
        val presence = when {
            input -> if (inputOptional) Presence.Optional else Presence.Required
            output -> if (outputOptional) Presence.Optional else Presence.Required
            else -> Presence.Required
        }
        val declaredAccess = outputProperty?.let(::declaredAccess)
            ?: inputProperty?.let(::declaredAccess)
        val access = when (declaredAccess) {
            JsonProperty.Access.READ_ONLY -> Access.ReadOnly
            JsonProperty.Access.WRITE_ONLY -> Access.WriteOnly
            else -> null
        }
        val fieldDirection = when {
            output && input -> Direction.Both
            output -> Direction.Output
            else -> Direction.Input
        }
        return IrField(
            wire = property.name,
            ident = words(property.name),
            ty = ref(
                property.primaryType,
                fieldDirection,
                wireHints(
                    format = members.firstNotNullOfOrNull { it.getAnnotation(JsonFormat::class.java) },
                    stringSerializer = members.any { it.isStringSerializer() },
                    nullable = nullable,
                ),
            ),
            presence = presence,
            default = (inputProperty?.metadata?.defaultValue ?: outputProperty?.metadata?.defaultValue)
                ?.takeIf { it.isNotEmpty() }
                ?.let { FieldDefault.Value(JsonPrimitive(it)) }
                ?: FieldDefault.Unspecified,
            access = access,
            doc = members.firstNotNullOfOrNull {
                it.getAnnotation(JsonPropertyDescription::class.java)
                    ?.value
                    ?.takeIf(String::isNotEmpty)
            }?.let { Doc(description = it) },
        )
    }

    private fun members(property: BeanPropertyDefinition): List<AnnotatedMember> = listOfNotNull(
        property.primaryMember,
        property.field,
        property.getter,
        property.setter,
        property.constructorParameter,
    )

    private fun declaredAccess(property: BeanPropertyDefinition): JsonProperty.Access? =
        members(property).firstNotNullOfOrNull {
            it.getAnnotation(JsonProperty::class.java)?.access
        }

    private fun enumType(id: String, beanType: Class<*>, direction: Direction): TypeDef {
        val variants = beanType.enumConstants.map { constant ->
            val name = (constant as Enum<*>).name
            val wire = beanType.getField(name)
                .getAnnotation(JsonProperty::class.java)
                ?.value
                ?.takeIf { it.isNotEmpty() }
                ?: name
            Variant(
                wire = wire,
                ident = words(wire),
                payload = VariantPayload.UnitPayload,
            )
        }
        return TypeDef(
            name = typeName(beanType),
            direction = direction,
            definition = TypeKind.EnumType(
                repr = EnumRepr.StringRepr,
                variants = variants.sortedBy { it.wire },
            ),
            origin = id,
        )
    }

    private fun unionType(id: String, beanType: Class<*>, direction: Direction): TypeDef {
        val info = typeInfo(beanType)
        val repr = representation(beanType, info)
        val subtypes = subtypes(beanType)
        val variants = subtypes.mapNotNull { (wire, subtype) ->
            if (subtype == beanType) return@mapNotNull null
            variant(subtype, wire, direction)
        }.sortedBy { it.wire }
        if (variants.isEmpty()) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Warning,
                code = "SPRING-W006",
                message = "no variants were found for the polymorphic type ${beanType.name}",
            )
        }
        return TypeDef(
            name = typeName(beanType),
            direction = direction,
            definition = TypeKind.EnumType(repr = repr, variants = variants),
            origin = id,
        )
    }

    private fun representation(beanType: Class<*>, info: JsonTypeInfo?): EnumRepr {
        if (info == null) return EnumRepr.UntaggedRepr
        if (info.use != JsonTypeInfo.Id.NAME) {
            diagnostics += Diagnostic(
                severity = DiagnosticSeverity.Error,
                code = "SPRING-E010",
                message = "the type ${beanType.name} uses ${info.use} type information; only Id.NAME is represented",
            )
            return EnumRepr.UntaggedRepr
        }
        return when (info.include) {
            JsonTypeInfo.As.PROPERTY, JsonTypeInfo.As.EXISTING_PROPERTY ->
                EnumRepr.InternalRepr(info.property.ifEmpty { "@type" })
            JsonTypeInfo.As.WRAPPER_OBJECT -> EnumRepr.ExternalRepr
            JsonTypeInfo.As.EXTERNAL_PROPERTY -> {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Warning,
                    code = "SPRING-W007",
                    message = "the type ${beanType.name} uses external type information; the content member is recorded as value",
                )
                EnumRepr.AdjacentRepr(info.property.ifEmpty { "@type" }, "value")
            }
            JsonTypeInfo.As.WRAPPER_ARRAY -> {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Error,
                    code = "SPRING-E011",
                    message = "the type ${beanType.name} uses wrapper-array type information, which the IR cannot represent",
                )
                EnumRepr.UntaggedRepr
            }
            JsonTypeInfo.As.NOTHING -> EnumRepr.UntaggedRepr
        }
    }

    private fun subtypes(beanType: Class<*>): List<Pair<String, Class<*>>> {
        val explicit = classInfo(beanType).getAnnotation(JsonSubTypes::class.java)
            ?.value
            ?.map { type ->
                val subtype = type.value.java
                type.name.ifEmpty { subtype.simpleName } to subtype
            }
            .orEmpty()
        if (explicit.isNotEmpty()) return explicit
        val sealed = runCatching { beanType.kotlin.isSealed }.getOrDefault(false)
        if (!sealed) return emptyList()
        val result = mutableListOf<Pair<String, Class<*>>>()
        collectSealed(beanType, result)
        return result
    }

    private fun collectSealed(beanType: Class<*>, result: MutableList<Pair<String, Class<*>>>) {
        val subclasses = runCatching { beanType.kotlin.sealedSubclasses.map { it.java } }.getOrDefault(emptyList())
        for (subclass in subclasses) {
            if (runCatching { subclass.kotlin.isSealed }.getOrDefault(false)) {
                collectSealed(subclass, result)
            } else if (!subclass.isInterface && !java.lang.reflect.Modifier.isAbstract(subclass.modifiers)) {
                val wire = subclass.getAnnotation(JsonTypeName::class.java)
                    ?.value
                    ?.takeIf { it.isNotEmpty() }
                    ?: subclass.simpleName
                result += wire to subclass
            }
        }
    }

    private fun variant(beanType: Class<*>, wire: String, direction: Direction): Variant {
        val source = valueSource(beanType)
        val payload = when {
            runCatching { beanType.kotlin.objectInstance }.getOrNull() != null ->
                VariantPayload.UnitPayload
            source != null -> VariantPayload.NewtypePayload(ref(source.type, direction, source.hints))
            else -> {
                val fields = fields(beanType, direction)
                if (fields.isEmpty()) {
                    VariantPayload.UnitPayload
                } else {
                    VariantPayload.StructPayload(fields)
                }
            }
        }
        return Variant(wire = wire, ident = words(wire), payload = payload)
    }

    private data class ValueSource(val type: JavaType, val hints: WireHints)

    private fun valueSource(beanType: Class<*>): ValueSource? {
        var current: Class<*>? = beanType
        while (current != null && current != Any::class.java) {
            current.declaredMethods.firstOrNull { method ->
                method.parameterCount == 0 &&
                    !java.lang.reflect.Modifier.isStatic(method.modifiers) &&
                    method.isAnnotationPresent(JsonValue::class.java)
            }?.let { method ->
                return ValueSource(
                    type = mapper.typeFactory.constructType(method.genericReturnType),
                    hints = wireHints(method, false),
                )
            }
            current.declaredFields.firstOrNull { field ->
                !java.lang.reflect.Modifier.isStatic(field.modifiers) &&
                    field.isAnnotationPresent(JsonValue::class.java)
            }?.let { field ->
                return ValueSource(
                    type = mapper.typeFactory.constructType(field.genericType),
                    hints = wireHints(field, false),
                )
            }
            current = current.superclass
        }
        return null
    }

    private fun classStringSerializer(beanType: Class<*>): Boolean =
        beanType.getAnnotation(com.fasterxml.jackson.databind.annotation.JsonSerialize::class.java)
            ?.using?.java == ToStringSerializer::class.java

    private fun openness(beanType: Class<*>, direction: Direction): ObjectOpenness {
        if (direction == Direction.Output) return ObjectOpenness.PreserveOpaque
        val ignoreUnknown = classInfo(beanType)
            .getAnnotation(JsonIgnoreProperties::class.java)
            ?.ignoreUnknown == true
        val failOnUnknown = mapper.deserializationConfig.isEnabled(
            DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES,
        )
        val open = ignoreUnknown || !failOnUnknown
        if (direction == Direction.Both) {
            if (!open) {
                diagnostics += Diagnostic(
                    severity = DiagnosticSeverity.Warning,
                    code = "SPRING-W008",
                    message = "the type ${beanType.name} rejects unknown input members but also has an output direction; it is recorded as preserve_opaque",
                )
            }
            return ObjectOpenness.PreserveOpaque
        }
        return if (open) ObjectOpenness.PreserveOpaque else ObjectOpenness.Reject
    }

    private fun skipsNull(beanType: Class<*>): Boolean {
        val include = classInfo(beanType).getAnnotation(JsonInclude::class.java)?.value
            ?: return false
        return include == JsonInclude.Include.NON_NULL ||
            include == JsonInclude.Include.NON_ABSENT ||
            include == JsonInclude.Include.NON_EMPTY ||
            include == JsonInclude.Include.NON_DEFAULT
    }

    private fun classInfo(beanType: Class<*>): AnnotatedClass = annotatedClasses.getOrPut(beanType.name) {
        mapper.serializationConfig
            .introspect(mapper.typeFactory.constructType(beanType))
            .classInfo
    }

    private fun typeInfo(beanType: Class<*>): JsonTypeInfo? = typeInfoCache.getOrPut(beanType.name) {
        classInfo(beanType).getAnnotation(JsonTypeInfo::class.java)
    }

    private fun isSealed(beanType: Class<*>): Boolean =
        runCatching { beanType.kotlin.isSealed }.getOrDefault(false)

    private fun intType(bits: Int, signed: Boolean, hints: WireHints): InlineType =
        InlineType.IntType(bits = bits, signed = signed, wire = integerWire(hints))

    private fun integerWire(hints: WireHints): IntegerWire {
        val asString = hints.stringWire ||
            hints.shape == JsonFormat.Shape.STRING ||
            mapper.serializationConfig.isEnabled(
                com.fasterxml.jackson.core.JsonGenerator.Feature.WRITE_NUMBERS_AS_STRINGS,
                mapper.factory,
            )
        return if (asString) IntegerWire.String else IntegerWire.Number
    }

    private fun decimalWire(hints: WireHints): DecimalWire {
        val asString = hints.stringWire ||
            hints.shape == JsonFormat.Shape.STRING ||
            mapper.serializationConfig.isEnabled(
                com.fasterxml.jackson.core.JsonGenerator.Feature.WRITE_NUMBERS_AS_STRINGS,
                mapper.factory,
            )
        return if (asString) DecimalWire.String else DecimalWire.Number
    }

    private fun isTemporal(beanType: Class<*>): Boolean =
        beanType == Instant::class.java ||
            beanType == OffsetDateTime::class.java ||
            beanType == ZonedDateTime::class.java ||
            beanType == LocalDateTime::class.java ||
            beanType == LocalDate::class.java ||
            beanType == LocalTime::class.java ||
            beanType == Duration::class.java ||
            beanType == Date::class.java ||
            Date::class.java.isAssignableFrom(beanType)

    private fun temporalType(beanType: Class<*>, hints: WireHints): InlineType {
        val asNumber = when (hints.shape) {
            JsonFormat.Shape.STRING -> false
            JsonFormat.Shape.NUMBER, JsonFormat.Shape.NUMBER_INT, JsonFormat.Shape.NUMBER_FLOAT -> true
            else -> mapper.serializationConfig.isEnabled(SerializationFeature.WRITE_DATES_AS_TIMESTAMPS)
        }
        if (asNumber) {
            val bits = if (beanType == LocalDate::class.java) 32 else 64
            return intType(bits, true, hints.copy(shape = JsonFormat.Shape.NUMBER))
        }
        return InlineType.StringType(
            format = when (beanType) {
                LocalDate::class.java -> FieldFormat.Date
                LocalTime::class.java -> FieldFormat.Time
                Duration::class.java -> FieldFormat.Duration
                else -> FieldFormat.DateTime
            },
        )
    }

    private fun nullable(type: TypeRef, nullable: Boolean): TypeRef =
        if (nullable) TypeRef.Inline(InlineType.NullableType(type)) else type

    private fun combine(directions: Set<Direction>): Direction = when {
        Direction.Both in directions -> Direction.Both
        Direction.Input in directions && Direction.Output in directions -> Direction.Both
        Direction.Input in directions -> Direction.Input
        else -> Direction.Output
    }

    private data class PropertyView(
        val outputProperty: BeanPropertyDefinition? = null,
        val inputProperty: BeanPropertyDefinition? = null,
        val output: Boolean = false,
        val input: Boolean = false,
    )
}
