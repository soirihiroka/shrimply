# Configuration file for the Sphinx documentation builder.

import re

from pygments.lexer import RegexLexer, bygroups, include, words
from pygments.token import (
    Comment,
    Keyword,
    Name,
    Number,
    Operator,
    Punctuation,
    String,
    Text,
)

project = "Shrimply Documentation"
copyright = "Shrimply contributors"
author = "Shrimply contributors"

extensions = [
    "sphinx_tabs.tabs",
    "sphinxext.opengraph",
]
source_suffix = {".rst": "restructuredtext"}
templates_path = ["_templates"]
exclude_patterns = []

html_title = "Shrimply Documentation"
html_theme = "furo"
html_theme_options = {
    "light_css_variables": {
        "color-brand-primary": "#4a86cf",
        "color-brand-content": "#4a86cf",
    },
    "source_edit_link": "https://github.com/soirihiroka/shrimply/edit/main/docs/source/{filename}",
}
html_favicon = "_static/shrimply-symbolic.svg"
html_static_path = ["_static"]
html_css_files = ["gnome.css"]
html_show_copyright = 0
html_show_sphinx = 0
show_source = 0

ogp_image = "_static/shrimply.svg"


def _string_state(quote: str, token, *, triple: bool = False, raw: bool = False):
    escaped = re.escape(quote)
    end = escaped * 3 if triple else f"{escaped}|\\n"
    excluded = f"\\\\{escaped}" + ("" if raw else "{}") + ("" if triple else "\\n")
    body = f"[^{excluded}]+" + (f"|{escaped}(?!{escaped}{escaped})" if triple else "")
    rules = [(end, token, "#pop"), include("escapes")]
    if not raw:
        rules.append(include("interpolation"))
    rules.append((body, token))
    return rules


class SanimLexer(RegexLexer):
    name = "Sanim"
    aliases = ["sanim"]
    filenames = ["*.sanim"]

    tokens = {
        "root": [
            (r"\s+", Text.Whitespace),
            (r"#[^\n]*", Comment.Single),
            # Raw and normal strings
            (r'\br"""', String.Double, "raw-triple-double"),
            (r'"""', String.Double, "triple-double"),
            (r"\br'''", String.Single, "raw-triple-single"),
            (r"'''", String.Single, "triple-single"),
            (r'\br"', String.Double, "raw-double"),
            (r"\br'", String.Single, "raw-single"),
            (r'"', String.Double, "double"),
            (r"'", String.Single, "single"),
            # XML-style native markup (not generic type arguments like read<List<T>>)
            (
                r"(</)([A-Z][A-Za-z0-9_]*)(\s*>)",
                bygroups(Punctuation, Name.Tag, Punctuation),
            ),
            (
                r"(?<![A-Za-z0-9_])(<)([A-Z][A-Za-z0-9_]*)(?=\s|/?>)",
                bygroups(Punctuation, Name.Tag),
                "tag",
            ),
            # Declarations
            (
                r"\b(fn)(\s*)(<)([^<>\n]+)(>)(\s+)([A-Za-z_]\w*)",
                bygroups(
                    Keyword.Declaration,
                    Text.Whitespace,
                    Punctuation,
                    Keyword.Type,
                    Punctuation,
                    Text.Whitespace,
                    Name.Function,
                ),
            ),
            (
                r"\b(fn)(\s+)([A-Za-z_]\w*)",
                bygroups(Keyword.Declaration, Text.Whitespace, Name.Function),
            ),
            (
                r"\b(fn)(\s*)(<)([^<>\n]+)(>)",
                bygroups(
                    Keyword.Declaration,
                    Text.Whitespace,
                    Punctuation,
                    Keyword.Type,
                    Punctuation,
                ),
            ),
            (r"\bfn\b", Keyword.Declaration),
            (
                r"\b(type|enum)(\s+)([A-Za-z_]\w*)",
                bygroups(Keyword.Declaration, Text.Whitespace, Name.Class),
            ),
            # Keywords and constants
            (
                words(
                    (
                        "let",
                        "mut",
                        "sig",
                        "reflect",
                        "import",
                        "as",
                        "if",
                        "else",
                        "for",
                        "in",
                        "return",
                        "match",
                        "case",
                        "and",
                        "or",
                        "not",
                        "repeat",
                    ),
                    prefix=r"\b",
                    suffix=r"\b",
                ),
                Keyword,
            ),
            (
                words(("true", "false", "None", "forever"), prefix=r"\b", suffix=r"\b"),
                Keyword.Constant,
            ),
            # Numbers with duration / layout / angle units
            (r"(?<![\w.])(?:\d+(?:\.\d+)?(?:ms|s|m))+(?!\w)", Number.Float),
            (r"(?<![\w.])\d+(?:\.\d+)?(?:pt|px|deg|fps|%)(?!\w)", Number.Float),
            (r"(?<![\w.])\d+\.\d+(?:[eE][+-]?\d+)?(?!\w)", Number.Float),
            (r"(?<![\w.])\d+(?:[eE][+-]?\d+)?(?!\w)", Number.Integer),
            # Method and function calls
            (r"(?<=\.)[a-z_][a-z0-9_]*(?=\s*\()", Name.Function),
            (r"\b[A-Z][A-Za-z0-9_]*(?=\s*(?:<[^>\n]+>)?\s*\()", Name.Class),
            (r"\b[a-z_][A-Za-z0-9_]*(?=\s*(?:<[^>\n]+>)?\s*\()", Name.Builtin),
            # Constants and types
            (r"\b[A-Z][A-Z0-9_]+\b", Name.Constant),
            (r"\b[A-Z][A-Za-z0-9_]*\b", Name.Class),
            # Format specifiers in inline interpolation (e.g. {count:.0f} in <Label>)
            (
                r"(:)((?:\.[0-9]+|[0-9]+)[a-zA-Z%]+)(?=\})",
                bygroups(Punctuation, String.Interpol),
            ),
            # Operators and punctuation
            (r"\*\*|->|[+*/%=!<>|&-]+", Operator),
            (r"[:,.;{}\[\]()]", Punctuation),
            (r"[a-z_][A-Za-z0-9_]*", Name),
        ],
        "escapes": [
            (r"\\(?:[\\'\"nrt0]|u\{[a-fA-F0-9]+\})", String.Escape),
        ],
        "interpolation": [
            (r"\{\{|\}\}", String.Escape),
            (r"\{", String.Interpol, "interp-expr"),
        ],
        "interp-expr": [
            (r":[^}\n]+", String.Interpol),
            (r"\}", String.Interpol, "#pop"),
            include("root"),
        ],
        "raw-triple-double": _string_state('"', String.Double, triple=True, raw=True),
        "triple-double": _string_state('"', String.Double, triple=True),
        "raw-triple-single": _string_state("'", String.Single, triple=True, raw=True),
        "triple-single": _string_state("'", String.Single, triple=True),
        "raw-double": _string_state('"', String.Double, raw=True),
        "raw-single": _string_state("'", String.Single, raw=True),
        "double": _string_state('"', String.Double),
        "single": _string_state("'", String.Single),
        "tag": [
            (r"\s+", Text.Whitespace),
            (r"/?>", Punctuation, "#pop"),
            (r"#[^\n]*", Comment.Single),
            (r'"', String.Double, "double"),
            (r"'", String.Single, "single"),
            (r"\{", Punctuation, "tag-expr"),
            (r"\b[A-Za-z_]\w*(?=\s*=)", Name.Attribute),
            (r"=", Operator),
            (r"\b[A-Za-z_][A-Za-z0-9_.%\-]*", Name.Constant),
        ],
        "tag-expr": [
            (r"\}", Punctuation, "#pop"),
            include("root"),
        ],
    }


def setup(app):
    app.add_lexer("sanim", SanimLexer)
