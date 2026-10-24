package com.pons.spring

import com.pons.ir.Document

fun interface PonsDocumentLowerer {
    fun lower(document: Document): Document
}
