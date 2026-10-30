package main

import (
	"fmt"
	"strings"
)

func renderOperationAll(builder *strings.Builder, plan *operationPlan) {
	pagination := plan.Pagination
	names := pagination.Locals
	fmt.Fprintf(builder, "\nfunc (c *Client) %s(%s, yield func(%s) error) error {\n", pagination.AllName, parameterList(plan), pagination.ItemType)
	if paginationUsesBody(pagination) {
		renderPaginationBodyBase(builder, plan, names)
	}
	initial := plan.QueryParam
	if pagination.Cursor.Carrier == "body" {
		initial = names.bodyBase
	}
	fmt.Fprintf(builder, "\t%s := %s\n", names.cursor, selectorExpression(initial, pagination.Cursor.Selector))
	if pagination.PageSize != nil {
		fmt.Fprintf(builder, "\t%s := true\n", names.first)
	}
	builder.WriteString("\tfor {\n")
	if paginationUsesQuery(pagination) {
		fmt.Fprintf(builder, "\t\t%s := %s\n", names.query, plan.QueryParam)
	}
	if paginationUsesBody(pagination) {
		fmt.Fprintf(builder, "\t\t%s := %s\n", names.body, names.bodyBase)
	}
	if pagination.PageSize != nil {
		fmt.Fprintf(builder, "\t\tif %s {\n", names.first)
		target := names.query
		if pagination.PageSize.Carrier == "body" {
			target = names.body
		}
		renderPageSizeDefault(builder, target, pagination.PageSize, names)
		builder.WriteString("\t\t}\n")
		fmt.Fprintf(builder, "\t\t%s = false\n", names.first)
	}
	target := names.query
	if pagination.Cursor.Carrier == "body" {
		target = names.body
	}
	fmt.Fprintf(builder, "\t\t%s = %s\n", selectorExpression(target, pagination.Cursor.Selector), names.cursor)
	fmt.Fprintf(builder, "\t\t%s, %s := c.%s(%s)\n", names.page, names.callErr, plan.Name, paginationArguments(plan))
	fmt.Fprintf(builder, "\t\tif %s != nil {\n\t\t\treturn %s\n\t\t}\n", names.callErr, names.callErr)
	renderPaginationItems(builder, plan, names)
	renderPaginationNext(builder, plan, names)
	builder.WriteString("\t}\n")
	builder.WriteString("}\n")
}

func paginationUsesQuery(pagination *paginationPlan) bool {
	return pagination.Cursor.Carrier == "query" ||
		(pagination.PageSize != nil && pagination.PageSize.Carrier == "query")
}

func paginationUsesBody(pagination *paginationPlan) bool {
	return pagination.Cursor.Carrier == "body" ||
		(pagination.PageSize != nil && pagination.PageSize.Carrier == "body")
}

func renderPaginationBodyBase(builder *strings.Builder, plan *operationPlan, names paginationLocals) {
	body := plan.Body
	baseType := strings.TrimPrefix(body.GoType, "*")
	if body.GoType == baseType {
		fmt.Fprintf(builder, "\t%s := %s\n", names.bodyBase, body.Param)
		return
	}
	fmt.Fprintf(builder, "\t%s := %s{}\n", names.bodyBase, baseType)
	fmt.Fprintf(builder, "\tif %s != nil {\n\t\t%s = *%s\n\t}\n", body.Param, names.bodyBase, body.Param)
}

func renderPageSizeDefault(builder *strings.Builder, target string, size *paginationSizePlan, names paginationLocals) {
	field := selectorExpression(target, size.Selector)
	value := fmt.Sprintf("%s(%d)", size.BaseType, size.Default)
	if !size.Pointer {
		fmt.Fprintf(builder, "\t\t\tif %s == 0 {\n", field)
		fmt.Fprintf(builder, "\t\t\t\t%s = %s\n", field, value)
		builder.WriteString("\t\t\t}\n")
		return
	}
	fmt.Fprintf(builder, "\t\t\tif %s == nil {\n", field)
	fmt.Fprintf(builder, "\t\t\t\t%s := %s\n", names.pageSize, value)
	fmt.Fprintf(builder, "\t\t\t\t%s = &%s\n", field, names.pageSize)
	builder.WriteString("\t\t\t}\n")
}

func paginationArguments(plan *operationPlan) string {
	pagination := plan.Pagination
	parts := []string{"ctx"}
	for _, param := range plan.Params {
		switch param.Kind {
		case "query":
			if paginationUsesQuery(pagination) {
				parts = append(parts, pagination.Locals.query)
			} else {
				parts = append(parts, param.Name)
			}
		case "body":
			if paginationUsesBody(pagination) {
				if strings.HasPrefix(plan.Body.GoType, "*") {
					parts = append(parts, "&"+pagination.Locals.body)
				} else {
					parts = append(parts, pagination.Locals.body)
				}
			} else {
				parts = append(parts, param.Name)
			}
		default:
			parts = append(parts, param.Name)
		}
	}
	return strings.Join(parts, ", ")
}

func renderPaginationItems(builder *strings.Builder, plan *operationPlan, names paginationLocals) {
	pagination := plan.Pagination
	items := paginationItemsExpression(names.page, plan.Success.Pointer, pagination.Items)
	if pagination.ItemsPointer {
		fmt.Fprintf(builder, "\t\tif %s != nil {\n", items)
		fmt.Fprintf(builder, "\t\t\tfor _, %s := range *%s {\n", names.item, items)
		renderPaginationYield(builder, names, "\t\t\t\t")
		builder.WriteString("\t\t\t}\n")
		builder.WriteString("\t\t}\n")
		return
	}
	fmt.Fprintf(builder, "\t\tfor _, %s := range %s {\n", names.item, items)
	renderPaginationYield(builder, names, "\t\t\t")
	builder.WriteString("\t\t}\n")
}

func renderPaginationYield(builder *strings.Builder, names paginationLocals, indent string) {
	fmt.Fprintf(builder, "%sif %s := yield(%s); %s != nil {\n", indent, names.yieldErr, names.item, names.yieldErr)
	fmt.Fprintf(builder, "%s\treturn %s\n", indent, names.yieldErr)
	fmt.Fprintf(builder, "%s}\n", indent)
}

func renderPaginationNext(builder *strings.Builder, plan *operationPlan, names paginationLocals) {
	pagination := plan.Pagination
	fmt.Fprintf(builder, "\t\t%s := %s\n", names.next, paginationNextExpression(names.page, plan.Success.Pointer, pagination.Next.Selector))
	if pagination.Next.Pointer {
		fmt.Fprintf(builder, "\t\tif %s == nil {\n\t\t\treturn nil\n\t\t}\n", names.next)
		fmt.Fprintf(builder, "\t\tif *%s == %s {\n\t\t\treturn nil\n\t\t}\n", names.next, emptyScalarLiteral(pagination.Next.Kind))
	} else {
		fmt.Fprintf(builder, "\t\tif %s == %s {\n\t\t\treturn nil\n\t\t}\n", names.next, emptyScalarLiteral(pagination.Next.Kind))
	}
	value := names.next
	if pagination.Next.Pointer {
		value = "*" + names.next
	}
	converted := fmt.Sprintf("%s(%s)", pagination.Cursor.BaseType, value)
	if pagination.Cursor.Pointer {
		fmt.Fprintf(builder, "\t\t%s := %s\n", names.nextValue, converted)
		fmt.Fprintf(builder, "\t\t%s = &%s\n", names.cursor, names.nextValue)
		return
	}
	fmt.Fprintf(builder, "\t\t%s = %s\n", names.cursor, converted)
}

func paginationItemsExpression(page string, named bool, selector []string) string {
	if len(selector) > 0 {
		return selectorExpression(page, selector)
	}
	if named {
		return "*" + page
	}
	return page
}

func paginationNextExpression(page string, named bool, selector []string) string {
	if len(selector) > 0 {
		return selectorExpression(page, selector)
	}
	if named {
		return "*" + page
	}
	return page
}

func selectorExpression(base string, selector []string) string {
	if len(selector) == 0 {
		return base
	}
	return base + "." + strings.Join(selector, ".")
}

func emptyScalarLiteral(kind string) string {
	if kind == "int" {
		return "0"
	}
	return `""`
}
