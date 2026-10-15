package com.pons.conformance

import com.pons.ir.Direction
import com.pons.ir.Doc
import com.pons.ir.Document
import com.pons.ir.ErrorContent
import com.pons.ir.ErrorResponse
import com.pons.ir.ErrorStatus
import com.pons.ir.Field
import com.pons.ir.InlineType
import com.pons.ir.NamedRef
import com.pons.ir.ObjectOpenness
import com.pons.ir.Operation
import com.pons.ir.PathSegment
import com.pons.ir.Presence
import com.pons.ir.TypeDef
import com.pons.ir.TypeKind
import com.pons.ir.TypeRef
import com.pons.ir.VariantPayload
import com.pons.spring.PonsDocumentLowerer
import org.springframework.stereotype.Component

private const val REJECTION_ID = "com.pons.conformance.RequestRejection"
private const val VIOLATION_ID = "com.pons.conformance.Violation"

@Component
class ConformanceDocumentLowerer : PonsDocumentLowerer {
    override fun lower(document: Document): Document =
        addRequestRejection(removeInferredFieldAccess(document))

    private fun removeInferredFieldAccess(document: Document): Document {
        // TODO: the Spring frontend infers read_only and write_only field access from direction, while the Axum frontend leaves access absent; drop this once the starter stops inferring it.
        return document.copy(
            types = document.types.mapValues { (_, definition) ->
                definition.copy(definition = withoutAccess(definition.definition))
            },
        )
    }

    private fun withoutAccess(kind: TypeKind): TypeKind = when (kind) {
        is TypeKind.StructType -> kind.copy(fields = kind.fields.map { it.copy(access = null) })
        is TypeKind.EnumType -> kind.copy(
            variants = kind.variants.map { variant ->
                when (val payload = variant.payload) {
                    is VariantPayload.StructPayload -> variant.copy(
                        payload = payload.copy(
                            fields = payload.fields.map { it.copy(access = null) },
                        ),
                    )
                    else -> variant
                }
            },
        )
        is TypeKind.NewtypeType -> kind
    }

    private fun addRequestRejection(document: Document): Document {
        // TODO: the Spring frontend does not declare the 422 request rejection the Axum frontend adds to operations with validated request inputs; drop this once the starter declares it.
        val missing = document.operations.filter { operation ->
            hasRequestInput(operation) &&
                operation.errors.none { it.status == ErrorStatus.Exact(422) }
        }
        if (missing.isEmpty()) return document
        val types = document.types +
            (REJECTION_ID to rejectionType()) +
            (VIOLATION_ID to violationType())
        return document.copy(
            types = types,
            operations = document.operations.map { operation ->
                if (operation in missing) {
                    operation.copy(errors = operation.errors + rejectionError())
                } else {
                    operation
                }
            },
        )
    }

    private fun hasRequestInput(operation: Operation): Boolean =
        operation.body != null ||
            operation.query != null ||
            operation.headers.isNotEmpty() ||
            operation.path.any { it is PathSegment.Param }

    private fun rejectionError(): ErrorResponse = ErrorResponse(
        status = ErrorStatus.Exact(422),
        name = listOf("request", "rejected"),
        content = ErrorContent.Json,
        body = TypeRef.Named(NamedRef(REJECTION_ID)),
        retryable = false,
        doc = Doc(summary = "The request does not satisfy the published request contract."),
    )

    private fun rejectionType(): TypeDef = TypeDef(
        name = listOf("violation", "report"),
        direction = Direction.Output,
        definition = TypeKind.StructType(
            fields = listOf(
                Field(
                    wire = "violations",
                    ident = listOf("violations"),
                    ty = TypeRef.Inline(
                        InlineType.ArrayType(TypeRef.Named(NamedRef(VIOLATION_ID))),
                    ),
                    presence = Presence.Required,
                ),
            ),
            openness = ObjectOpenness.Reject,
        ),
    )

    private fun violationType(): TypeDef = TypeDef(
        name = listOf("violation"),
        direction = Direction.Output,
        definition = TypeKind.StructType(
            fields = listOf(
                Field(
                    wire = "path",
                    ident = listOf("path"),
                    ty = TypeRef.Inline(InlineType.StringType()),
                    presence = Presence.Required,
                ),
                Field(
                    wire = "reason",
                    ident = listOf("reason"),
                    ty = TypeRef.Inline(InlineType.StringType()),
                    presence = Presence.Required,
                ),
            ),
            openness = ObjectOpenness.Reject,
        ),
    )
}
