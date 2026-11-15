"""Render the generated client runtime and credential helpers."""

from __future__ import annotations

from . import naming
from .analysis import Analysis
from .render import literal


def render_runtime_types() -> list[str]:
    return [
        "@dataclass(slots=True)",
        "class Request:",
        "    method: str",
        "    url: str",
        "    headers: dict[str, str]",
        "    body: bytes | None",
        "    timeout: float | None = None",
        "",
        "",
        "Transport = Callable[[Request], tuple[int, Mapping[str, str | list[str]], bytes]]",
        "",
        "",
        "@dataclass(slots=True)",
        "class Response(Generic[_T]):",
        "    status: int",
        "    body: _T",
        "    headers: dict[str, str]",
        "",
        "",
        "@dataclass(frozen=True, slots=True)",
        "class _Scheme:",
        "    kind: str",
        "    location: str = \"\"",
        "    name: str = \"\"",
        "",
        "",
        "def _header_map(message: Any) -> dict[str, list[str]]:",
        "    values: dict[str, list[str]] = {}",
        "    for name, value in message.items():",
        "        values.setdefault(name, []).append(value)",
        "    return values",
        "",
        "",
        "def _urllib_transport(request: Request) -> tuple[int, Mapping[str, str | list[str]], bytes]:",
        "    prepared = urllib.request.Request(",
        "        request.url,",
        "        data=request.body,",
        "        headers=request.headers,",
        "        method=request.method,",
        "    )",
        "    try:",
        "        with urllib.request.urlopen(prepared, timeout=request.timeout) as response:",
        "            return response.status, _header_map(response.headers), response.read()",
        "    except urllib.error.HTTPError as error:",
        "        return error.code, _header_map(error.headers), error.read()",
    ]


def render_schemes(analysis: Analysis) -> list[str]:
    schemes = analysis.document.auth_schemes
    if not schemes:
        return ["_SCHEMES: dict[str, _Scheme] = {}"]
    lines = ["_SCHEMES: dict[str, _Scheme] = {"]
    for scheme_id in sorted(schemes):
        scheme = schemes[scheme_id]
        if scheme.kind in {"api_key", "cookie_session"}:
            lines.append(
                f"    {literal(scheme_id)}: _Scheme({literal(scheme.kind)}, "
                f"{literal(scheme.location)}, {literal(scheme.name)}),"
            )
        else:
            lines.append(f"    {literal(scheme_id)}: _Scheme({literal(scheme.kind)}),")
    lines.append("}")
    return lines


def provides_constants(analysis: Analysis) -> dict[str, str]:
    registry = naming.NameRegistry({"_PROVIDES"})
    constants: dict[str, str] = {}
    for plan in analysis.operations:
        if plan.provides is None:
            continue
        base = "_PROVIDES_" + naming.screaming([naming.words(plan.id)])
        constants[plan.id] = registry.claim(base)
    return constants


def locator_text(locator: object) -> str:
    return f"({literal(locator.source)}, {literal(locator.value)})"


def render_provides(analysis: Analysis, constants: dict[str, str]) -> list[str]:
    lines = []
    for plan in analysis.operations:
        if plan.provides is None:
            continue
        name = constants[plan.id]
        expires = (
            "None"
            if plan.provides.expires_at is None
            else locator_text(plan.provides.expires_at)
        )
        lines.extend(
            [
                f"{name}: tuple[str, tuple[str, str], tuple[str, str] | None] = (",
                f"    {literal(plan.provides.scheme)},",
                f"    {locator_text(plan.provides.value)},",
                f"    {expires},",
                ")",
            ]
        )
    return lines


def render_client_helpers() -> list[str]:
    return [
        "",
        "    def _perform(",
        "        self,",
        "        method: str,",
        "        path: str,",
        "        *,",
        "        query: list[tuple[str, str]],",
        "        headers: dict[str, str],",
        "        body: bytes | None,",
        "        alternatives: list[list[str]],",
        "        required: bool,",
        "    ) -> tuple[int, dict[str, str], bytes]:",
        "        if not self.base_url:",
        "            raise ConfigurationError(\"base_url is empty\")",
        "        merged = dict(self.headers)",
        "        merged.update(headers)",
        "        auth_headers, auth_pairs = self._authorize(alternatives, required)",
        "        merged.update(auth_headers)",
        "        pairs = list(query)",
        "        pairs.extend(auth_pairs)",
        "        url = self.base_url + path",
        "        if pairs:",
        "            url += \"?\" + urllib.parse.urlencode(pairs, quote_via=urllib.parse.quote)",
        "        request = Request(",
        "            method=method,",
        "            url=url,",
        "            headers=merged,",
        "            body=body,",
        "            timeout=self.timeout,",
        "        )",
        "        try:",
        "            status, response_headers, response_body = self.transport(request)",
        "        except PonsError:",
        "            raise",
        "        except Exception as error:",
        "            raise TransportError(str(error)) from error",
        "        return status, _wire.normalize_headers(response_headers), bytes(response_body)",
        "",
        "    def _authorize(",
        "        self,",
        "        alternatives: list[list[str]],",
        "        required: bool,",
        "    ) -> tuple[dict[str, str], list[tuple[str, str]]]:",
        "        if not alternatives:",
        "            return {}, []",
        "        missing: list[str] = []",
        "        for alternative in alternatives:",
        "            attachments: list[tuple[_Scheme, object]] = []",
        "            absent: list[str] = []",
        "            for scheme_id in alternative:",
        "                scheme = _SCHEMES[scheme_id]",
        "                credential = self._credential(scheme_id)",
        "                if credential is None:",
        "                    absent.append(scheme_id)",
        "                else:",
        "                    attachments.append((scheme, credential))",
        "            if absent:",
        "                missing.append(\"missing credentials for \" + \", \".join(absent))",
        "                continue",
        "            headers: dict[str, str] = {}",
        "            pairs: list[tuple[str, str]] = []",
        "            try:",
        "                for scheme, credential in attachments:",
        "                    self._attach(scheme, credential, headers, pairs)",
        "            except ConfigurationError as error:",
        "                missing.append(str(error))",
        "                continue",
        "            return headers, pairs",
        "        if required:",
        "            raise ConfigurationError(\"; \".join(missing))",
        "        return {}, []",
        "",
        "    def _credential(self, scheme_id: str) -> object | None:",
        "        value = self._auth.get(scheme_id)",
        "        if value is None:",
        "            value = self.credentials.get(scheme_id)",
        "        if value is None or value == \"\":",
        "            return None",
        "        return value",
        "",
        "    def _attach(",
        "        self,",
        "        scheme: _Scheme,",
        "        credential: object,",
        "        headers: dict[str, str],",
        "        pairs: list[tuple[str, str]],",
        "    ) -> None:",
        "        if scheme.kind in {\"bearer\", \"o_auth2\"}:",
        "            if not isinstance(credential, str) or not credential:",
        "                raise ConfigurationError(",
        "                    f\"the {scheme.kind} credential is not a token\"",
        "                )",
        "            headers[\"Authorization\"] = \"Bearer \" + credential",
        "            return",
        "        if scheme.kind == \"basic\":",
        "            headers[\"Authorization\"] = \"Basic \" + self._basic(credential)",
        "            return",
        "        if scheme.kind == \"api_key\":",
        "            value = str(credential)",
        "            if scheme.location == \"header\":",
        "                headers[scheme.name] = value",
        "            elif scheme.location == \"query\":",
        "                pairs.append((scheme.name, value))",
        "            elif scheme.location == \"cookie\":",
        "                self._cookie(headers, scheme.name, value)",
        "            else:",
        "                raise ConfigurationError(",
        "                    f\"the api key scheme uses the {scheme.location!r} carrier\"",
        "                )",
        "            return",
        "        if scheme.kind == \"cookie_session\":",
        "            self._cookie(headers, scheme.name, str(credential))",
        "            return",
        "        raise ConfigurationError(f\"the {scheme.kind} scheme is not supported\")",
        "",
        "    def _cookie(self, headers: dict[str, str], name: str, value: str) -> None:",
        "        item = f\"{name}={value}\"",
        "        current = headers.get(\"Cookie\")",
        "        headers[\"Cookie\"] = f\"{current}; {item}\" if current else item",
        "",
        "    def _basic(self, credential: object) -> str:",
        "        if isinstance(credential, Mapping):",
        "            username = credential.get(\"username\", credential.get(\"user\"))",
        "            password = credential.get(\"password\")",
        "        elif isinstance(credential, str) and \":\" in credential:",
        "            username, password = credential.split(\":\", 1)",
        "        else:",
        "            raise ConfigurationError(\"basic auth needs a username and password\")",
        "        if not isinstance(username, str) or not isinstance(password, str):",
        "            raise ConfigurationError(\"basic auth needs a username and password\")",
        "        token = base64.b64encode(f\"{username}:{password}\".encode(\"utf-8\"))",
        "        return token.decode(\"ascii\")",
        "",
        "    def _decode(",
        "        self,",
        "        raw: bytes,",
        "        status: int,",
        "        headers: dict[str, str],",
        "        media: str,",
        "        codec: Callable[[object, str], object],",
        "        validate: Callable[[object, str], object] | None = None,",
        "    ) -> object:",
        "        if not _wire.content_type_matches(headers, media):",
        "            actual = _wire.header(headers, \"content-type\") or \"missing\"",
        "            raise DecodeError(",
        "                f\"expected content type {media!r}, found {actual!r}\",",
        "                path=\"headers.content-type\",",
        "                status=status,",
        "            )",
        "        try:",
        "            value = json.loads(raw)",
        "        except (TypeError, ValueError, UnicodeDecodeError) as error:",
        "            raise DecodeError(str(error), status=status) from error",
        "        try:",
        "            decoded = codec(value, \"\")",
        "        except DecodeError as error:",
        "            raise error.with_status(status)",
        "        if validate is not None:",
        "            try:",
        "                issue = validate(decoded, \"\")",
        "            except Exception as error:",
        "                raise DecodeError(",
        "                    f\"the validator failed: {error}\", status=status",
        "                ) from error",
        "            if issue is not None:",
        "                raise DecodeError(",
        "                    issue.message, path=issue.path, status=status",
        "                )",
        "        return decoded",
        "",
        "    def _check_header(",
        "        self,",
        "        headers: dict[str, str],",
        "        name: str,",
        "        status: int,",
        "        path: str,",
        "        required: bool,",
        "        validate: Callable[[object, str], object] | None = None,",
        "    ) -> None:",
        "        value = _wire.header(headers, name)",
        "        if value is None:",
        "            if required:",
        "                raise DecodeError(\"missing response header\", path=path, status=status)",
        "            return",
        "        if validate is None:",
        "            return",
        "        try:",
        "            issue = validate(value, path)",
        "        except Exception as error:",
        "            raise DecodeError(",
        "                f\"the validator failed: {error}\", path=path, status=status",
        "            ) from error",
        "        if issue is not None:",
        "            raise DecodeError(issue.message, path=issue.path, status=status)",
        "",
        "    def _store_provides(",
        "        self,",
        "        provides: tuple[str, tuple[str, str], tuple[str, str] | None],",
        "        headers: dict[str, str],",
        "        body: bytes,",
        "    ) -> None:",
        "        scheme, value_locator, expires_locator = provides",
        "        self.credentials[scheme] = _wire.provide_value(",
        "            value_locator, headers, body",
        "        )",
        "        if expires_locator is not None:",
        "            self._expirations[scheme] = _wire.locator_value(",
        "                expires_locator, headers, body",
        "            )",
    ]
