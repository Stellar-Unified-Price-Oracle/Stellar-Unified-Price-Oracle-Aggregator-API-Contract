"""Evidence redaction for logs and incident artifacts (#687).

One filter in front of every artifact the operator tooling produces, with no
pass-through mode. See ``docs/security/evidence-redaction.md``.
"""

from services.evidence_redaction.redact import (
    RULES,
    Finding,
    RedactionDisabled,
    RedactionResult,
    RedactionRule,
    Redactor,
    redact_stream,
    redact_text,
    render_report,
)

__all__ = [
    "RULES",
    "Finding",
    "RedactionDisabled",
    "RedactionResult",
    "RedactionRule",
    "Redactor",
    "redact_stream",
    "redact_text",
    "render_report",
]
