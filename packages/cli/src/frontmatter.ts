/**
 * Pulls a single field out of a Markdown file's YAML frontmatter. Not a real YAML parser — handles
 * a plain scalar or a folded (`>`) / literal (`|`) block scalar, which covers every skill and agent
 * file in practice.
 */
export function extractFrontmatterField(body: string, key: string): string | undefined {
  const lines = body.split(/\r?\n/);
  const pattern = new RegExp(`^${key}:\\s*(.*)$`);
  for (let i = 0; i < lines.length; i++) {
    const m = pattern.exec(lines[i]);
    if (!m) {
      continue;
    }
    const rest = m[1].trim();
    if (rest === '>' || rest === '>-' || rest === '|' || rest === '|-') {
      // A folded/literal block scalar: gather the more-indented lines that follow, folded onto one line.
      const indent = /^\s*/.exec(lines[i])![0].length;
      const collected: string[] = [];
      for (let j = i + 1; j < lines.length; j++) {
        const line = lines[j];
        if (line.trim() === '') {
          continue;
        }
        if (/^\s*/.exec(line)![0].length <= indent) {
          break;
        }
        collected.push(line.trim());
      }
      return collected.join(' ');
    }
    const value = rest.replace(/^['"]|['"]$/g, '');
    return value || undefined;
  }
  return undefined;
}

/** Extracts `name` and `description` from a Markdown file's `---`-delimited YAML frontmatter block. */
export function parseNameAndDescription(text: string): { name?: string; description?: string } {
  const match = /^---\r?\n([\s\S]*?)\r?\n---/.exec(text);
  if (!match) {
    return {};
  }
  const body = match[1];
  return { name: extractFrontmatterField(body, 'name'), description: extractFrontmatterField(body, 'description') };
}
