package main

import (
	"fmt"
	"strings"
)

func renderClient(a *analysis) string {
	var builder strings.Builder
	writeFileHeader(&builder, a.pkg, []string{"fmt", "net/http", "net/url", "strings"})
	fmt.Fprintf(&builder, `
type Client struct {
	BaseURL     string
	HTTPClient  *http.Client
	BearerToken string
	APIKey      string
}

const apiKeyHeader = %q
const apiKeyQuery = %q

func (c *Client) httpClient() *http.Client {
	if c.HTTPClient != nil {
		return c.HTTPClient
	}
	return http.DefaultClient
}

func (c *Client) endpoint(path string, query url.Values) (string, error) {
	base := strings.TrimRight(c.BaseURL, "/")
	if base == "" {
		return "", &ConfigurationError{Message: "Client.BaseURL is empty"}
	}
	endpoint := base + path
	if encoded := query.Encode(); encoded != "" {
		endpoint += "?" + encoded
	}
	return endpoint, nil
}

func (c *Client) authorize(request *http.Request, bearer, apiKey, required bool) error {
	if bearer {
		if c.BearerToken == "" {
			if required {
				return &ConfigurationError{Message: "Client.BearerToken is empty"}
			}
		} else {
			request.Header.Set("Authorization", "Bearer "+c.BearerToken)
		}
	}
	if apiKey {
		if c.APIKey == "" {
			if required {
				return &ConfigurationError{Message: "Client.APIKey is empty"}
			}
		} else if apiKeyHeader != "" {
			request.Header.Set(apiKeyHeader, c.APIKey)
		} else if apiKeyQuery != "" {
			values := request.URL.Query()
			values.Set(apiKeyQuery, c.APIKey)
			request.URL.RawQuery = values.Encode()
		}
	}
	return nil
}

func scalarText(value any) string {
	return fmt.Sprint(value)
}
`, a.apiKeyHeader, a.apiKeyQuery)
	return builder.String()
}
