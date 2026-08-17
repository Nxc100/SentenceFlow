/**
 * Markdown — AI 输出的轻量排版(答疑/周点评)。
 * 只支持教学回答实际会出现的子集:段落、无序/有序列表、**加粗**、`行内代码`;
 * 逐 token 构建 React 节点,不经 innerHTML,无注入面。
 * 刻意不引第三方 Markdown 库:体积与供应链成本都不值得。
 */

import type { ReactNode } from "react";

/** 行内标记:**bold** 与 `code` */
function renderInline(text: string, keyBase: string): ReactNode[] {
  const out: ReactNode[] = [];
  // 交替切分:先按 code,再在剩余段内按 bold
  const codeParts = text.split(/(`[^`]+`)/g);
  codeParts.forEach((part, ci) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
      out.push(
        <code key={`${keyBase}-c${ci}`} className="sf-md__code">
          {part.slice(1, -1)}
        </code>,
      );
      return;
    }
    const boldParts = part.split(/(\*\*[^*]+\*\*)/g);
    boldParts.forEach((seg, bi) => {
      if (seg.startsWith("**") && seg.endsWith("**") && seg.length > 4) {
        out.push(<strong key={`${keyBase}-b${ci}-${bi}`}>{seg.slice(2, -2)}</strong>);
      } else if (seg) {
        out.push(seg);
      }
    });
  });
  return out;
}

type Block =
  | { kind: "p"; lines: string[] }
  | { kind: "ul"; items: string[] }
  | { kind: "ol"; items: string[] };

function parseBlocks(text: string): Block[] {
  const blocks: Block[] = [];
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trimEnd();
    const ul = /^\s*[-*•]\s+(.*)$/.exec(line);
    const ol = /^\s*\d+[.、)]\s+(.*)$/.exec(line);
    const last = blocks[blocks.length - 1];
    if (ul) {
      if (last?.kind === "ul") last.items.push(ul[1]!);
      else blocks.push({ kind: "ul", items: [ul[1]!] });
    } else if (ol) {
      if (last?.kind === "ol") last.items.push(ol[1]!);
      else blocks.push({ kind: "ol", items: [ol[1]!] });
    } else if (line.trim() === "") {
      // 空行 = 段落边界(连续空行折叠)
      if (last?.kind === "p" && last.lines.length > 0) blocks.push({ kind: "p", lines: [] });
    } else {
      if (last?.kind === "p") last.lines.push(line);
      else blocks.push({ kind: "p", lines: [line] });
    }
  }
  return blocks.filter((b) => (b.kind === "p" ? b.lines.length > 0 : b.items.length > 0));
}

export interface MarkdownProps {
  text: string;
  className?: string;
}

export function Markdown({ text, className }: MarkdownProps) {
  const blocks = parseBlocks(text);
  return (
    <div className={["sf-md", className].filter(Boolean).join(" ")}>
      {blocks.map((block, i) => {
        if (block.kind === "ul" || block.kind === "ol") {
          const List = block.kind === "ul" ? "ul" : "ol";
          return (
            <List key={i} className="sf-md__list">
              {block.items.map((item, j) => (
                <li key={j}>{renderInline(item, `${i}-${j}`)}</li>
              ))}
            </List>
          );
        }
        return (
          <p key={i} className="sf-md__p">
            {block.lines.map((line, j) => (
              <span key={j}>
                {j > 0 && <br />}
                {renderInline(line, `${i}-${j}`)}
              </span>
            ))}
          </p>
        );
      })}
    </div>
  );
}
