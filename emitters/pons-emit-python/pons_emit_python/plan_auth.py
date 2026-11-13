"""Authentication schemes and credential provisioning for the Python emitter."""

from __future__ import annotations

from typing import TYPE_CHECKING

from .codes import DOCUMENT, UNSUPPORTED

if TYPE_CHECKING:
    from .analysis import Analysis
    from .document import AuthScheme, Operation
    from .plan_operations import OperationPlan


class CredentialPlanner:
    def __init__(self, analysis: Analysis) -> None:
        self.analysis = analysis

    def build_auth(
        self, plan: OperationPlan, operation: Operation, subject: str
    ) -> None:
        if any(not alternative for alternative in operation.auth.alternatives):
            self.analysis.add(
                DOCUMENT,
                subject,
                "an auth alternative is empty, so it names no scheme",
            )
        for alternative in operation.auth.alternatives:
            for scheme_id in alternative:
                scheme = self.analysis.document.auth_schemes.get(scheme_id)
                if scheme is None:
                    self.analysis.add(
                        DOCUMENT,
                        subject,
                        f"the auth scheme {scheme_id} is not declared",
                    )
                    continue
                self.validate_scheme(scheme, scheme_id, subject)
        plan.alternatives = [
            list(alternative) for alternative in operation.auth.alternatives
        ]
        plan.required_auth = not operation.auth.optional and bool(plan.alternatives)

    def validate_scheme(
        self, scheme: AuthScheme, scheme_id: str, subject: str
    ) -> None:
        if scheme.kind in {"bearer", "basic", "o_auth2"}:
            return
        if scheme.kind == "api_key":
            if scheme.location not in {"header", "query", "cookie"}:
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the api key scheme {scheme_id} is carried in {scheme.location!r}",
                )
            elif not scheme.name:
                self.analysis.add(
                    DOCUMENT,
                    subject,
                    f"the api key scheme {scheme_id} has no carrier name",
                )
            return
        if scheme.kind == "cookie_session":
            if not scheme.name:
                self.analysis.add(
                    DOCUMENT,
                    subject,
                    f"the cookie session scheme {scheme_id} has no cookie name",
                )
            return
        self.analysis.add(
            UNSUPPORTED,
            subject,
            f"the auth scheme {scheme_id} uses the {scheme.kind} kind, which this emitter does not support",
        )

    def build_provides(
        self, plan: OperationPlan, operation: Operation, subject: str
    ) -> None:
        provides = plan.provides
        if provides is None:
            return
        scheme = self.analysis.document.auth_schemes.get(provides.scheme)
        if scheme is None:
            self.analysis.add(
                DOCUMENT,
                subject,
                f"the provided scheme {provides.scheme} is not declared",
            )
            return
        self.validate_scheme(scheme, provides.scheme, subject)
        for locator in [provides.value, provides.expires_at]:
            if locator is None:
                continue
            if locator.source == "body":
                pointer = locator.value
                if pointer and not pointer.startswith("/"):
                    self.analysis.add(
                        DOCUMENT,
                        subject,
                        f"the credential body pointer {pointer!r} must be empty or start with '/'",
                    )
                index = 0
                while index < len(pointer):
                    if pointer[index] == "~":
                        if index + 1 >= len(pointer) or pointer[index + 1] not in "01":
                            self.analysis.add(
                                DOCUMENT,
                                subject,
                                "the credential body pointer has an invalid escape",
                            )
                            break
                        index += 2
                        continue
                    index += 1
            elif not locator.value:
                self.analysis.add(
                    DOCUMENT,
                    subject,
                    f"the credential {locator.source} locator has no name",
                )
