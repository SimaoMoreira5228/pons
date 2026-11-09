package com.pons.ir

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.doubleOrNull
import java.math.BigDecimal
import java.math.BigInteger
import java.util.regex.Pattern
import java.util.regex.PatternSyntaxException

data class Violation(
    val path: String,
    val reason: String,
)

fun validateValue(
    document: Document,
    ty: TypeRef,
    direction: Direction,
    value: JsonElement,
): List<Violation> {
    val validator = ValueValidator(document, direction)
    validator.check(ty, "", value)
    return validator.violations
}

private const val ALIAS = "com.pons.alias/v1"
private val u128Max: BigInteger = BigInteger.ONE.shiftLeft(128).subtract(BigInteger.ONE)

private sealed interface Exact {
    data class Integer(val negative: Boolean, val magnitude: BigInteger) : Exact

    data object Fraction : Exact

    data object Beyond : Exact
}

private data class Member(
    val wire: String,
    val aliases: List<String>,
    val ty: TypeRef,
    val presence: Presence,
    val access: Access?,
    val filled: Boolean,
) {
    fun names(aliased: Boolean): List<String> =
        if (aliased && aliases.isNotEmpty()) listOf(wire) + aliases else listOf(wire)

    fun accepts(name: String, aliased: Boolean): Boolean = names(aliased).contains(name)
}

private class ValueValidator(
    private val document: Document,
    private val direction: Direction,
) {
    val violations = mutableListOf<Violation>()
    private val newtypes = mutableSetOf<String>()

    fun check(ty: TypeRef, path: String, value: JsonElement) {
        when (ty) {
            is TypeRef.Named -> named(ty.ref, path, value)
            is TypeRef.Inline -> inline(ty.type, path, value)
        }
    }

    private fun named(named: NamedRef, path: String, value: JsonElement) {
        val definition = document.types[named.id]
        if (definition == null) {
            return reject(Violation(path, "unknown type ${named.id}"))
        }
        if (newtypes.contains(named.id)) {
            return reject(
                Violation(path, "type ${named.id} cannot be resolved because newtypes form a cycle"),
            )
        }
        when (val kind = definition.definition) {
            is TypeKind.NewtypeType -> {
                newtypes.add(named.id)
                check(kind.inner, path, value)
                newtypes.remove(named.id)
            }
            is TypeKind.StructType -> objectValue(kind.fields, kind.flattened, kind.openness, path, value)
            is TypeKind.EnumType -> enumeration(kind.repr, kind.variants, named.variant, path, value)
        }
    }

    private fun inline(type: InlineType, path: String, value: JsonElement) {
        when (type) {
            InlineType.BoolType -> expect(path, "a boolean", value, JsonElement::isBoolean)
            InlineType.NullType -> expect(path, "null", value, JsonElement::isNull)
            InlineType.AnyType -> {}
            is InlineType.OpaqueType -> reject(
                Violation(path, "opaque type ${type.codec} is not supported by the request validator"),
            )
            is InlineType.IntType -> integer(type, path, value)
            is InlineType.FloatType -> floating(type, path, value)
            is InlineType.DecimalType -> decimal(type.wire, path, value)
            is InlineType.StringType -> text(type, path, value)
            is InlineType.BytesType -> bytes(type.encoding, path, value)
            is InlineType.NullableType -> {
                if (value !is JsonNull) check(type.inner, path, value)
            }
            is InlineType.ArrayType -> sequence(type.item, type.unique, path, value)
            is InlineType.MapType -> mapping(type.key, type.value, path, value)
            is InlineType.TupleType -> tuple(type.items, path, value)
            is InlineType.StructType -> objectValue(type.fields, type.flattened, type.openness, path, value)
        }
    }

    private fun integer(type: InlineType.IntType, path: String, value: JsonElement) {
        val exact = when (type.wire) {
            IntegerWire.Number -> {
                if (!expect(path, "a number", value, JsonElement::isNumber)) return
                when (val parsed = exactInteger(value)) {
                    is Exact.Integer -> parsed
                    Exact.Fraction -> {
                        reject(
                            Violation(path, "expected an exact integer, found $value"),
                        )
                        return
                    }
                    Exact.Beyond -> {
                        reject(Violation(path, "integer is outside the ${sign(type.signed)}${type.bits} range"))
                        return
                    }
                }
            }
            IntegerWire.String -> {
                val text = stringContent(value)
                if (text == null) {
                    typeViolation(path, "an integer string", value)
                    return
                }
                if (!canonicalInteger(text, type.signed)) {
                    reject(Violation(path, "expected an exact integer, found $text"))
                    return
                }
                val negative = text.startsWith('-')
                val digits = if (negative) text.substring(1) else text
                val magnitude = BigInteger(digits)
                if (magnitude > u128Max) {
                    reject(Violation(path, "integer is outside the ${sign(type.signed)}${type.bits} range"))
                    return
                }
                Exact.Integer(negative && magnitude != BigInteger.ZERO, magnitude)
            }
        }
        val actual = if (exact.negative) exact.magnitude.negate() else exact.magnitude
        val problem = when {
            outsideNative(actual, type.bits, type.signed) ->
                "integer is outside the ${sign(type.signed)}${type.bits} range"
            type.minimum != null && bound(actual, type.minimum) < 0 ->
                "integer is below its minimum of ${type.minimum}"
            type.maximum != null && bound(actual, type.maximum) > 0 ->
                "integer is above its maximum of ${type.maximum}"
            else -> multipleProblem(exact.magnitude, type.multipleOf)
        }
        if (problem != null) reject(Violation(path, problem))
    }

    private fun floating(type: InlineType.FloatType, path: String, value: JsonElement) {
        if (!number(path, value)) return
        val number = (value as? JsonPrimitive)?.content?.toBigDecimalOrNull() ?: return
        if (type.bits == 32 && number.abs() > BigDecimal(Float.MAX_VALUE.toDouble())) {
            reject(Violation(path, "number is outside the f${type.bits} range"))
            return
        }
        val problem = minimumProblem(number, type.minimum)
            ?: maximumProblem(number, type.maximum)
            ?: exclusiveMinimumProblem(number, type.exclusiveMinimum)
            ?: exclusiveMaximumProblem(number, type.exclusiveMaximum)
            ?: multipleProblem(number, type.multipleOf)
        if (problem != null) reject(Violation(path, problem))
    }

    private fun decimal(wire: DecimalWire, path: String, value: JsonElement) {
        when (wire) {
            DecimalWire.Number -> number(path, value)
            DecimalWire.String -> {
                val text = stringContent(value)
                if (text == null) return typeViolation(path, "a decimal string", value)
                if (!isDecimal(text)) contentViolation(path, "a decimal string", value)
            }
        }
    }

    private fun number(path: String, value: JsonElement): Boolean {
        if (!expect(path, "a number", value, JsonElement::isNumber)) return false
        val number = (value as? JsonPrimitive)?.doubleOrNull
        if (number == null || !number.isFinite()) {
            typeViolation(path, "a finite number", value)
            return false
        }
        return true
    }

    private fun text(type: InlineType.StringType, path: String, value: JsonElement) {
        val text = stringContent(value) ?: return typeViolation(path, "a string", value)
        val found = text.codePointCount(0, text.length).toLong()
        val minimum = type.minLength
        val maximum = type.maxLength
        if (minimum != null && found < minimum) {
            reject(
                Violation(path, "string is shorter than the $minimum code points it allows"),
            )
            return
        }
        if (maximum != null && found > maximum) {
            reject(
                Violation(path, "string is longer than the $maximum code points it allows"),
            )
            return
        }
        val pattern = type.pattern ?: return
        val compiled = try {
            Pattern.compile(pattern)
        } catch (error: PatternSyntaxException) {
            return reject(
                Violation(
                    path,
                    "pattern $pattern cannot be evaluated because ${error.description}",
                ),
            )
        }
        if (!compiled.matcher(text).find()) {
            reject(Violation(path, "string does not match the pattern $pattern"))
        }
    }

    private fun bytes(encoding: ByteEncoding, path: String, value: JsonElement) {
        if (encoding == ByteEncoding.Array) {
            val items = value as? JsonArray ?: return typeViolation(path, "an array of bytes", value)
            for ((index, item) in items.withIndex()) {
                val itemPath = child(path, index.toString())
                when (val exact = exactInteger(item)) {
                    is Exact.Integer -> {
                        if (exact.negative || exact.magnitude > BigInteger.valueOf(255)) {
                            contentViolation(itemPath, "a byte between 0 and 255", item)
                        }
                    }
                    Exact.Fraction -> typeViolation(itemPath, "a byte between 0 and 255", item)
                    Exact.Beyond -> contentViolation(itemPath, "a byte between 0 and 255", item)
                }
            }
            return
        }
        val text = stringContent(value) ?: return typeViolation(path, "encoded bytes", value)
        val shaped = when (encoding) {
            ByteEncoding.Hex -> text.length % 2 == 0 && text.all { it.isAsciiHexDigit() }
            ByteEncoding.Base64 -> isBase64(text, false)
            ByteEncoding.Base64url -> isBase64(text, true)
            ByteEncoding.Array -> true
        }
        if (!shaped) {
            val reason = when (encoding) {
                ByteEncoding.Hex -> "invalid hexadecimal bytes"
                ByteEncoding.Base64 -> "invalid base64 bytes"
                ByteEncoding.Base64url -> "invalid base64url bytes"
                ByteEncoding.Array -> "invalid byte array"
            }
            reject(Violation(path, reason))
        }
    }

    private fun sequence(item: TypeRef, unique: Boolean?, path: String, value: JsonElement) {
        val items = value as? JsonArray ?: return typeViolation(path, "an array", value)
        val seen = mutableListOf<JsonElement>()
        for ((index, element) in items.withIndex()) {
            descend(item, child(path, index.toString()), element)
            if (unique != true) continue
            val first = seen.indexOfFirst { sameItem(it, element) }
            if (first >= 0) {
                reject(Violation(path, "array item $index repeats item $first"))
            } else {
                seen.add(element)
            }
        }
    }

    private fun mapping(key: TypeRef, item: TypeRef, path: String, value: JsonElement) {
        val entries = value as? JsonObject ?: return typeViolation(path, "an object", value)
        for ((name, entry) in entries) {
            val entryPath = child(path, name)
            descend(key, entryPath, JsonPrimitive(name))
            descend(item, entryPath, entry)
        }
    }

    private fun tuple(items: List<TypeRef>, path: String, value: JsonElement) {
        val elements = value as? JsonArray ?: return typeViolation(path, "an array", value)
        if (elements.size != items.size) {
            return reject(
                Violation(path, "expected a tuple of ${items.size} items, found ${elements.size}"),
            )
        }
        for ((index, pair) in items.zip(elements).withIndex()) {
            descend(pair.first, child(path, index.toString()), pair.second)
        }
    }

    private fun objectValue(
        fields: List<Field>,
        flattened: List<TypeRef>,
        openness: ObjectOpenness,
        path: String,
        value: JsonElement,
    ) {
        if (!expect(path, "an object", value, JsonElement::isObject)) return
        val entries = value as JsonObject
        val members = fields.mapTo(mutableListOf(), ::member)
        flattened.forEach { flattenedMember(it, mutableListOf(), members) }
        members.forEach { member(it, entries, path) }
        val aliased = direction == Direction.Input
        when (openness) {
            ObjectOpenness.Reject -> {
                for (name in entries.keys) {
                    if (members.none { it.accepts(name, aliased) }) {
                        reject(Violation(child(path, name), "unknown property $name"))
                    }
                }
            }
            is ObjectOpenness.PreserveTyped -> {
                val extra = opennessValue(openness.value, mutableListOf())
                for ((name, entry) in entries) {
                    if (members.none { it.accepts(name, aliased) }) {
                        descend(extra, child(path, name), entry)
                    }
                }
            }
            ObjectOpenness.PreserveOpaque -> {}
        }
    }

    private fun member(member: Member, entries: JsonObject, path: String) {
        val aliased = direction == Direction.Input
        val crosses = !(
            (direction == Direction.Input && member.access == Access.ReadOnly) ||
                (direction == Direction.Output && member.access == Access.WriteOnly)
            )
        if (!crosses) {
            val present = member.names(aliased).any { entries.containsKey(it) }
            if (present || (member.presence == Presence.Required && !member.filled)) {
                reject(
                    Violation(
                        child(path, member.wire),
                        "field ${member.wire} is ${accessName(member.access)} and cannot cross the ${directionName(direction)} boundary",
                    ),
                )
            }
            return
        }
        var first: Pair<String, JsonElement>? = null
        for (name in member.names(aliased)) {
            val value = entries[name] ?: continue
            if (first == null) {
                first = name to value
            } else {
                reject(
                    Violation(
                        child(path, name),
                        "field ${member.wire} is written twice, once as ${first.first}",
                    ),
                )
            }
        }
        val memberPath = child(path, member.wire)
        when {
            first != null -> descend(member.ty, memberPath, first.second)
            member.presence == Presence.Required ->
                reject(Violation(memberPath, "missing required field ${member.wire}"))
        }
    }

    private fun flattenedMember(
        ty: TypeRef,
        seen: MutableList<String>,
        members: MutableList<Member>,
    ) {
        val named = (ty as? TypeRef.Named)?.ref ?: return
        if (seen.contains(named.id)) return
        val definition = document.types[named.id] ?: return
        when (val kind = definition.definition) {
            is TypeKind.NewtypeType -> {
                seen.add(named.id)
                flattenedMember(kind.inner, seen, members)
                seen.remove(named.id)
            }
            is TypeKind.StructType -> kind.fields.mapTo(members, ::member)
            is TypeKind.EnumType -> {}
        }
    }

    private fun opennessValue(ty: TypeRef, seen: MutableList<String>): TypeRef {
        var current = ty
        while (true) {
            when (val inline = current) {
                is TypeRef.Inline -> {
                    if (inline.type is InlineType.MapType) {
                        return inline.type.value
                    }
                    return current
                }
                is TypeRef.Named -> {
                    if (seen.contains(inline.ref.id)) return current
                    val definition = document.types[inline.ref.id] ?: return current
                    val kind = definition.definition
                    if (kind is TypeKind.NewtypeType) {
                        seen.add(inline.ref.id)
                        current = kind.inner
                    } else {
                        return current
                    }
                }
            }
        }
    }

    private fun enumeration(
        repr: EnumRepr,
        variants: List<Variant>,
        selected: String?,
        path: String,
        value: JsonElement,
    ) {
        when (repr) {
            EnumRepr.UntaggedRepr -> {
                val candidates = variants.filter { selected == null || it.wire == selected }
                if (candidates.isEmpty()) {
                    return reject(
                        Violation(path, variantReason(selected, value.toString())),
                    )
                }
                for (variant in candidates) {
                    val mark = violations.size
                    payload(variant, path, value, true)
                    if (violations.size == mark) return
                    while (violations.size > mark) violations.removeAt(violations.lastIndex)
                }
                reject(Violation(path, "value matches none of the enum variants"))
            }
            EnumRepr.ExternalRepr -> external(variants, selected, path, value)
            is EnumRepr.InternalRepr -> {
                val variant = tagged(variants, selected, path, value, repr.tag) ?: return
                if (variant.payload is VariantPayload.TuplePayload) {
                    expect(path, "an object", value, JsonElement::isObject)
                    return
                }
                payload(variant, path, value, false)
            }
            is EnumRepr.AdjacentRepr -> {
                val variant = tagged(variants, selected, path, value, repr.tag) ?: return
                if (variant.payload is VariantPayload.UnitPayload) return
                val entries = value as? JsonObject ?: return
                val content = entries[repr.content]
                if (content == null) {
                    return reject(
                        Violation(
                            child(path, repr.content),
                            "missing required field ${repr.content}",
                        ),
                    )
                }
                payload(variant, child(path, repr.content), content, false)
            }
            EnumRepr.StringRepr -> {
                val variant = variant(variants, selected, path, value) ?: return
                payload(variant, path, value, false)
            }
        }
    }

    private fun external(
        variants: List<Variant>,
        selected: String?,
        path: String,
        value: JsonElement,
    ) {
        if (value is JsonPrimitive && value.isString) {
            val variant = variant(variants, selected, path, value) ?: return
            payload(variant, path, value, false)
            return
        }
        if (!expect(path, "an object", value, JsonElement::isObject)) return
        val entries = value as JsonObject
        if (entries.size != 1) {
            typeViolation(path, "an object with one variant", value)
            return
        }
        val (wire, payloadValue) = entries.entries.single()
        val variant = namedVariant(variants, selected, path, wire) ?: return
        payload(variant, child(path, wire), payloadValue, false)
    }

    private fun variant(
        variants: List<Variant>,
        selected: String?,
        path: String,
        value: JsonElement,
    ): Variant? {
        val wire = stringContent(value)
        if (wire == null) {
            typeViolation(path, "an enum variant name", value)
            return null
        }
        return namedVariant(variants, selected, path, wire)
    }

    private fun namedVariant(
        variants: List<Variant>,
        selected: String?,
        path: String,
        wire: String,
    ): Variant? {
        val variant = variants.firstOrNull { it.wire == wire }
        if (variant != null && (selected == null || selected == wire)) return variant
        reject(Violation(path, variantReason(selected, wire)))
        return null
    }

    private fun tagged(
        variants: List<Variant>,
        selected: String?,
        path: String,
        value: JsonElement,
        tag: String,
    ): Variant? {
        if (!expect(path, "an object", value, JsonElement::isObject)) return null
        val entries = value as JsonObject
        val tagValue = entries[tag]
        if (tagValue == null) {
            reject(Violation(child(path, tag), "missing required field $tag"))
            return null
        }
        val tagPath = child(path, tag)
        val wire = stringContent(tagValue)
        if (wire == null) {
            typeViolation(tagPath, "an enum variant name", tagValue)
            return null
        }
        return namedVariant(variants, selected, tagPath, wire)
    }

    private fun payload(variant: Variant, path: String, value: JsonElement, untagged: Boolean) {
        when (val payload = variant.payload) {
            VariantPayload.UnitPayload -> {
                if (untagged && value !is JsonNull) typeViolation(path, "null", value)
            }
            is VariantPayload.NewtypePayload -> descend(payload.ty, path, value)
            is VariantPayload.TuplePayload -> tuple(payload.items, path, value)
            is VariantPayload.StructPayload -> objectValue(
                payload.fields,
                emptyList(),
                ObjectOpenness.PreserveOpaque,
                path,
                value,
            )
        }
    }

    private fun descend(ty: TypeRef, path: String, value: JsonElement) {
        newtypes.clear()
        check(ty, path, value)
    }

    private fun reject(violation: Violation) {
        violations.add(violation)
    }

    private fun expect(
        path: String,
        expected: String,
        value: JsonElement,
        shape: (JsonElement) -> Boolean,
    ): Boolean {
        if (shape(value)) return true
        typeViolation(path, expected, value)
        return false
    }

    private fun typeViolation(path: String, expected: String, value: JsonElement) {
        reject(Violation(path, "expected $expected, found ${describe(value)}"))
    }

    private fun contentViolation(path: String, expected: String, value: JsonElement) {
        reject(Violation(path, "expected $expected, found $value"))
    }
}

private fun member(field: Field): Member {
    val aliases = (field.ext[ALIAS] as? JsonArray)
        ?.mapNotNull { (it as? JsonPrimitive)?.takeIf(JsonPrimitive::isString)?.content }
        .orEmpty()
    return Member(
        wire = field.wire,
        aliases = aliases,
        ty = field.ty,
        presence = field.presence,
        access = field.access,
        filled = field.default is FieldDefault.Value,
    )
}

private fun child(path: String, token: String): String =
    "$path/${token.replace("~", "~0").replace("/", "~1")}"

private fun describe(value: JsonElement): String = when (value) {
    JsonNull -> "null"
    is JsonArray -> "an array"
    is JsonObject -> "an object"
    is JsonPrimitive -> when {
        value.isString -> "a string"
        value.isBoolean -> "a boolean"
        else -> "a number"
    }
}

private fun JsonElement.isNull(): Boolean = this is JsonNull

private fun JsonElement.isBoolean(): Boolean = this is JsonPrimitive && isBoolean

private fun JsonElement.isNumber(): Boolean =
    this is JsonPrimitive && !isString && !isBoolean

private fun JsonElement.isObject(): Boolean = this is JsonObject

private val JsonPrimitive.isBoolean: Boolean
    get() = !isString && booleanOrNull != null

private fun stringContent(value: JsonElement): String? =
    (value as? JsonPrimitive)?.takeIf(JsonPrimitive::isString)?.content

private fun exactInteger(value: JsonElement): Exact {
    if (!value.isNumber()) return Exact.Fraction
    val text = value.toString()
    val parsed = try {
        BigDecimal(text)
    } catch (_: NumberFormatException) {
        return Exact.Fraction
    }
    val normalized = parsed.stripTrailingZeros()
    if (normalized.scale() > 0) return Exact.Fraction
    val magnitude = normalized.toBigIntegerExact()
    if (magnitude > u128Max) return Exact.Beyond
    return Exact.Integer(magnitude.signum() < 0, magnitude.abs())
}

private fun bound(actual: BigInteger, bound: JsonElement): Int {
    val value = exactInteger(bound)
    return if (value is Exact.Integer) {
        actual.compareTo(if (value.negative) value.magnitude.negate() else value.magnitude)
    } else {
        -1
    }
}

private fun numericBound(bound: JsonElement): BigDecimal? =
    (bound as? JsonPrimitive)?.takeIf(JsonPrimitive::isNumber)?.content?.toBigDecimalOrNull()

private fun minimumProblem(actual: BigDecimal, bound: JsonElement?): String? {
    if (bound == null) return null
    val minimum = numericBound(bound)
        ?: return "number constraint $bound is not a positive number"
    return if (actual < minimum) "number is below its minimum of $bound" else null
}

private fun maximumProblem(actual: BigDecimal, bound: JsonElement?): String? {
    if (bound == null) return null
    val maximum = numericBound(bound)
        ?: return "number constraint $bound is not a positive number"
    return if (actual > maximum) "number is above its maximum of $bound" else null
}

private fun exclusiveMinimumProblem(actual: BigDecimal, bound: JsonElement?): String? {
    if (bound == null) return null
    val minimum = numericBound(bound)
        ?: return "number constraint $bound is not a positive number"
    return if (actual <= minimum) "number is not above its exclusive minimum of $bound" else null
}

private fun exclusiveMaximumProblem(actual: BigDecimal, bound: JsonElement?): String? {
    if (bound == null) return null
    val maximum = numericBound(bound)
        ?: return "number constraint $bound is not a positive number"
    return if (actual >= maximum) "number is not below its exclusive maximum of $bound" else null
}

private fun multipleProblem(actual: BigDecimal, multipleOf: JsonElement?): String? {
    if (multipleOf == null) return null
    val step = numericBound(multipleOf)?.takeIf { it.signum() > 0 }
        ?: return "number constraint $multipleOf is not a positive number"
    return if (actual.remainder(step).signum() != 0) {
        "number is not a multiple of $multipleOf"
    } else {
        null
    }
}

private fun outsideNative(actual: BigInteger, bits: Int, signed: Boolean): Boolean {
    if (bits !in setOf(8, 16, 32, 64, 128)) return false
    val magnitude = BigInteger.ONE.shiftLeft(bits - 1)
    return when {
        signed -> actual < magnitude.negate() || actual > magnitude.subtract(BigInteger.ONE)
        else -> actual < BigInteger.ZERO || actual > magnitude.shiftLeft(1).subtract(BigInteger.ONE)
    }
}

private fun multipleProblem(magnitude: BigInteger, multipleOf: JsonElement?): String? {
    if (multipleOf == null) return null
    val step = exactInteger(multipleOf)
    if (step !is Exact.Integer || step.negative || step.magnitude == BigInteger.ZERO) {
        return "integer constraint $multipleOf is not an exact integer"
    }
    return if (magnitude.mod(step.magnitude) != BigInteger.ZERO) {
        "integer is not a multiple of $multipleOf"
    } else {
        null
    }
}

private fun canonicalInteger(text: String, signed: Boolean): Boolean {
    val digits = if (signed && text.startsWith('-')) text.substring(1) else text
    if (digits.isEmpty()) return false
    if (digits != "0" && digits.first() !in '1'..'9') return false
    return digits.all { it in '0'..'9' }
}

private fun isDecimal(text: String): Boolean =
    Regex("^-?(?:\\d+(?:\\.\\d*)?|\\.\\d+)(?:[eE][+-]?\\d+)?$").matches(text)

private fun isBase64(text: String, url: Boolean): Boolean {
    fun alphabet(byte: Char): Boolean = when (byte) {
        '-', '_' -> url
        '+', '/' -> !url
        else -> byte in 'A'..'Z' || byte in 'a'..'z' || byte in '0'..'9'
    }
    val body = text.trimEnd('=')
    val padding = text.length - body.length
    if (padding > 2) return false
    if (body.isEmpty()) return padding == 0
    if (!body.all(::alphabet)) return false
    val tail = body.length % 4
    return when (padding) {
        0 -> tail == 0 || url
        1 -> tail == 3
        2 -> tail == 2
        else -> false
    }
}

private fun Char.isAsciiHexDigit(): Boolean =
    this in '0'..'9' || this in 'a'..'f' || this in 'A'..'F'

private fun sameItem(left: JsonElement, right: JsonElement): Boolean {
    if (left is JsonPrimitive && right is JsonPrimitive && left.isNumber() && right.isNumber()) {
        val leftValue = left.content.toBigDecimalOrNull()
        val rightValue = right.content.toBigDecimalOrNull()
        return leftValue != null && rightValue != null && leftValue.compareTo(rightValue) == 0
    }
    return when {
        left is JsonArray && right is JsonArray ->
            left.size == right.size && left.zip(right).all { sameItem(it.first, it.second) }
        left is JsonObject && right is JsonObject ->
            left.size == right.size &&
                left.all { (name, value) -> right[name]?.let { sameItem(value, it) } == true }
        else -> left == right
    }
}

private fun sign(signed: Boolean): String = if (signed) "i" else "u"

private fun accessName(access: Access?): String = when (access) {
    Access.ReadOnly -> "read_only"
    Access.WriteOnly -> "write_only"
    Access.ReadWrite, null -> "read_write"
}

private fun directionName(direction: Direction): String = when (direction) {
    Direction.Input -> "input"
    Direction.Output -> "output"
    Direction.Both -> "both"
}

private fun variantReason(selected: String?, found: String): String =
    if (selected == null) "unknown enum variant $found" else "expected enum variant $selected, found $found"
