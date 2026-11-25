import React, { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Check, Copy } from "lucide-react";
import { createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import githubDarkDefault from "shiki/themes/github-dark-default.mjs";
import githubLightDefault from "shiki/themes/github-light-default.mjs";
import langBash from "shiki/langs/bash.mjs";
import langCss from "shiki/langs/css.mjs";
import langDiff from "shiki/langs/diff.mjs";
import langGo from "shiki/langs/go.mjs";
import langHtml from "shiki/langs/html.mjs";
import langJava from "shiki/langs/java.mjs";
import langJavascript from "shiki/langs/javascript.mjs";
import langJson from "shiki/langs/json.mjs";
import langJsx from "shiki/langs/jsx.mjs";
import langKotlin from "shiki/langs/kotlin.mjs";
import langMarkdown from "shiki/langs/markdown.mjs";
import langPython from "shiki/langs/python.mjs";
import langRust from "shiki/langs/rust.mjs";
import langSql from "shiki/langs/sql.mjs";
import langToml from "shiki/langs/toml.mjs";
import langTsx from "shiki/langs/tsx.mjs";
import langTypescript from "shiki/langs/typescript.mjs";
import langYaml from "shiki/langs/yaml.mjs";
import type { AppTheme } from "../state/useTheme";

let highlighterPromise: Promise<HighlighterCore> | null = null;
const loadedLangs = new Set<string>([
  "typescript",
  "tsx",
  "javascript",
  "jsx",
  "rust",
  "kotlin",
  "python",
  "bash",
  "sh",
  "json",
  "toml",
  "yaml",
  "css",
  "html",
  "markdown",
  "diff",
  "sql",
  "go",
  "java",
]);

function getShikiHighlighter(): Promise<HighlighterCore> {
  if (!highlighterPromise) {
    highlighterPromise = createHighlighterCore({
      themes: [githubDarkDefault, githubLightDefault],
      langs: [
        langTypescript,
        langTsx,
        langJavascript,
        langJsx,
        langRust,
        langKotlin,
        langPython,
        langBash,
        langJson,
        langToml,
        langYaml,
        langCss,
        langHtml,
        langMarkdown,
        langDiff,
        langSql,
        langGo,
        langJava,
      ],
      engine: createJavaScriptRegexEngine(),
    });
  }
  return highlighterPromise;
}

const LANG_ALIAS: Record<string, string> = {
  ts: "typescript",
  js: "javascript",
  py: "python",
  rs: "rust",
  kt: "kotlin",
  yml: "yaml",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  console: "bash",
  md: "markdown",
  jsonc: "json",
};

function normalizeLang(raw?: string): string {
  if (!raw) return "text";
  const lower = raw.trim().toLowerCase();
  return LANG_ALIAS[lower] ?? lower;
}

const CodeBlock: React.FC<{ language: string; code: string; theme: AppTheme }> = React.memo(
  ({ language, code, theme }) => {
    const [html, setHtml] = useState<string | null>(null);
    const [copied, setCopied] = useState(false);
    const lang = useMemo(() => normalizeLang(language), [language]);
    const lineCount = useMemo(() => code.split("\n").length, [code]);

    useEffect(() => {
      let active = true;
      if (lang === "text" || code.length > 60_000) {
        setHtml(null);
        return;
      }
      void getShikiHighlighter()
        .then(async (hl) => {
          if (!active) return;
          const resolved = loadedLangs.has(lang) ? lang : "text";
          if (resolved === "text") {
            setHtml(null);
            return;
          }
          const rendered = hl.codeToHtml(code, {
            lang: resolved,
            theme: theme === "light" ? "github-light-default" : "github-dark-default",
          });
          if (active) setHtml(rendered);
        })
        .catch(() => {
          if (active) setHtml(null);
        });
      return () => {
        active = false;
      };
    }, [code, lang, theme]);

    const handleCopy = () => {
      void navigator.clipboard.writeText(code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    };

    return (
      <div className="group/code my-3 border border-line-subtle bg-code-surface select-text">
        <div className="flex h-8 items-center justify-between border-b border-line-subtle bg-code-header px-3 font-mono text-[11px] text-ink-muted select-none">
          <div className="flex items-center gap-2">
            <span className="lowercase text-ink-2">{lang}</span>
            <span className="text-ink-faint">·</span>
            <span className="text-ink-muted">{lineCount} lines</span>
          </div>
          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1 px-1.5 py-0.5 text-[11px] text-ink-muted hover:bg-hover hover:text-ink-strong transition-colors cursor-pointer"
          >
            {copied ? (
              <Check className="h-3 w-3 text-success" />
            ) : (
              <Copy className="h-3 w-3" />
            )}
            <span>{copied ? "已复制" : "复制"}</span>
          </button>
        </div>
        {html ? (
          <div
            className="mux-shiki overflow-x-auto"
            dangerouslySetInnerHTML={{ __html: html }}
          />
        ) : (
          <pre className="m-0 overflow-x-auto bg-code-surface px-3.5 py-3 font-mono text-[12.5px] leading-[1.58] text-ink">
            <code>{code}</code>
          </pre>
        )}
      </div>
    );
  },
);

CodeBlock.displayName = "CodeBlock";

export const MarkdownView: React.FC<{
  text: string;
  theme: AppTheme;
  streaming?: boolean;
}> = React.memo(({ text, theme, streaming }) => {
  return (
    <div className="mux-markdown select-text">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          code({ className, children, ...props }) {
            const match = /language-([\w+-]+)/.exec(className || "");
            const raw = String(children).replace(/\n$/, "");
            const isBlock = Boolean(match) || raw.includes("\n");
            if (isBlock) {
              return <CodeBlock language={match?.[1] ?? "text"} code={raw} theme={theme} />;
            }
            return (
              <code className={className} {...props}>
                {children}
              </code>
            );
          },
          pre({ children }) {
            return <>{children}</>;
          },
          a({ href, children }) {
            return (
              <a
                href={href}
                onClick={(e) => {
                  e.preventDefault();
                  if (href) void window.muxlane.openExternal(href);
                }}
              >
                {children}
              </a>
            );
          },
        }}
      >
        {text}
      </ReactMarkdown>
      {streaming && (
        <span className="ml-1 inline-block h-3.5 w-1.5 animate-pulse bg-ink-2 align-middle" />
      )}
    </div>
  );
});

MarkdownView.displayName = "MarkdownView";
