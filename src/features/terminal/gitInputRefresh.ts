/**
 * 终端输入的行跟踪（T5.2 的"git 命令后刷新仓库状态"）。
 *
 * # 为什么在输入侧而不是输出侧
 *
 * `onData` 给的是**用户键入**的字节（不是回显），行缓冲不会被 shell 的
 * 着色/重绘污染。一个纯 `LineTracker` 就能还原"用户按 Enter 时那一行是什么"。
 *
 * # 已知边界（T5.3 会替换成完整的词法分析器）
 *
 * 这里只做刷新触发的轻量判定：不处理多行续行（PS 的 `>>`）、不处理粘贴中的
 * 控制序列——判定**宁可漏报不可误伤**（漏了就是状态晚几秒刷新，由文件监听
 * 兜底；误报是多刷一次查询，无伤大雅但吵）。
 */

/** 键入行跟踪器：把 onData 字节流还原成"逐条提交的命令行"。 */
export class LineTracker {
  private line = '';

  /** 消费一段键入，返回其中完整提交的行（按提交顺序）。 */
  feed(data: string): string[] {
    const completed: string[] = [];
    for (const char of data) {
      switch (char) {
        case '\r':
        case '\n':
          completed.push(this.line);
          this.line = '';
          break;
        case '\x7f': // backspace：删一个字符（近似；组合字符不管）
          this.line = this.line.slice(0, -1);
          break;
        case '\x03': // Ctrl+C：取消当前行
        case '\x15': // Ctrl+U：整行删除
          this.line = '';
          break;
        default:
          if (char >= ' ') {
            this.line += char;
          }
        // 其余控制字符（方向键转义序列的中间字节等）忽略——
        // 方向键补全历史会让本行失真，但对"是不是 git 命令"的判定
        // 影响可接受（见模块头的边界说明）
      }
      if (this.line.length > 2000) {
        this.line = this.line.slice(-2000);
      }
    }
    return completed;
  }
}

/** 是否是 git 命令（含 `git.exe`、前导空白；`legend`/`digit` 之类不算）。 */
export function isGitCommand(line: string): boolean {
  return /^\s*git(\.exe)?(\s|$)/i.test(line);
}
