// i18n-ignore-file
// 本文件是开发专用页面（PTY Spike 的 IPC 量测入口，路由只在 dev 构建注册），
// 文案面向开发者，整文件豁免 i18n:lint；正式终端 UI（T5.2）一律走 i18n key。
//
// 为什么这个页面存在：spike 示例验证的是后端管线，本页面接的是真实 Tauri
// 事件通道——"base64 载荷经 IPC 到前端"这一跳的吞吐与正确性只有真机可量。
// 结论记录在 docs/PTY-SPIKE.md。
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  createUtf8StreamDecoder,
  listenPtySpikeExit,
  listenPtySpikeOutput,
  ptySpikeClose,
  ptySpikeCreate,
  ptySpikeResize,
  ptySpikeThroughput,
  ptySpikeWrite,
  utf8ToBase64,
  type PtySpikeThroughput,
} from '@/lib/ipc';
import { Button } from '@/ui/components/button';
import { Input } from '@/ui/components/input';

/**
 * PTY Spike 面板：创建会话 → 发命令看输出 → 跑 10 万行吞吐量测试。
 * 前端计时（invoke 往返）与后端计时（收满 10 万行）并列展示，差值即 IPC 开销。
 */
export function PtySpikePanel() {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [program, setProgram] = useState<string>('');
  const [output, setOutput] = useState<string>('');
  const [eventCount, setEventCount] = useState(0);
  const [input, setInput] = useState('echo hello');
  const [throughput, setThroughput] = useState<
    (PtySpikeThroughput & { frontendMs: number }) | null
  >(null);
  const [error, setError] = useState<string | null>(null);

  const sessionIdRef = useRef<string | null>(null);
  const outputRef = useRef<HTMLPreElement>(null);

  // 订阅在挂载时建立、卸载时释放；用 ref 过滤当前会话，
  // 避免"切会话后旧会话的尾部输出串台"。
  useEffect(() => {
    const decodeChunk = createUtf8StreamDecoder();
    const unlistenOutput = listenPtySpikeOutput((payload) => {
      if (payload.id !== sessionIdRef.current) return;
      setEventCount((count) => count + 1);
      setOutput((current) => current + decodeChunk(payload.data));
    });
    const unlistenExit = listenPtySpikeExit((payload) => {
      if (payload.id !== sessionIdRef.current) return;
      setOutput((current) => current + `\n[exit code=${payload.code}]`);
    });
    return () => {
      void unlistenOutput.then((unlisten) => unlisten());
      void unlistenExit.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    outputRef.current?.scrollTo({ top: outputRef.current.scrollHeight });
  }, [output]);

  const handleCreate = useCallback(async () => {
    try {
      setError(null);
      const info = await ptySpikeCreate(120, 30);
      sessionIdRef.current = info.id;
      setSessionId(info.id);
      setProgram(info.program);
      setOutput('');
      setEventCount(0);
      setThroughput(null);
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  const handleSend = useCallback(async () => {
    if (!sessionIdRef.current) return;
    try {
      setError(null);
      // 输入以 \r 结尾（Enter）；cmd 分支在 spike 里同样接受 \r。
      await ptySpikeWrite(sessionIdRef.current, utf8ToBase64(`${input}\r`));
    } catch (reason) {
      setError(String(reason));
    }
  }, [input]);

  const handleThroughput = useCallback(async () => {
    if (!sessionIdRef.current) return;
    try {
      setError(null);
      const started = performance.now();
      const result = await ptySpikeThroughput(sessionIdRef.current);
      setThroughput({ ...result, frontendMs: Math.round(performance.now() - started) });
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  const handleClose = useCallback(async () => {
    if (!sessionIdRef.current) return;
    try {
      await ptySpikeClose(sessionIdRef.current);
    } finally {
      sessionIdRef.current = null;
      setSessionId(null);
      setProgram('');
    }
  }, []);

  return (
    <div className="flex h-full flex-col gap-3 p-4">
      <h1 className="text-lg font-semibold">PTY Spike（T5.1）— IPC 通道量测</h1>
      <p className="text-fg-muted text-sm">
        结论与数据见 docs/PTY-SPIKE.md。传输：base64 / 16ms 合并块。此页面与后端命令仅存在于
        开发构建。
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <Button onClick={() => void handleCreate()}>{sessionId ? '重建会话' : '创建会话'}</Button>
        <Button onClick={() => void handleThroughput()} disabled={!sessionId}>
          吞吐量测试（10 万行）
        </Button>
        <Button
          variant="secondary"
          onClick={() => void ptySpikeResize(sessionId ?? '', 100, 40)}
          disabled={!sessionId}
        >
          resize 100x40
        </Button>
        <Button variant="danger" onClick={() => void handleClose()} disabled={!sessionId}>
          关闭会话
        </Button>
        <span className="text-fg-muted text-sm">
          {sessionId
            ? `会话 ${sessionId} · shell: ${program} · 已收 ${eventCount} 个事件块`
            : '未创建会话'}
        </span>
      </div>

      {error ? <p className="text-danger text-sm">{error}</p> : null}

      {throughput ? (
        <div className="border-line bg-surface rounded border p-3 text-sm">
          <p>
            后端：{throughput.lines} 行 / {throughput.bytes} 字节 / {throughput.elapsedMs}ms （≈{' '}
            {Math.round(throughput.lines / (throughput.elapsedMs / 1000))} 行/s）
          </p>
          <p>前端 invoke 往返：{throughput.frontendMs}ms —— 差值≈IPC 序列化与事件分发开销</p>
        </div>
      ) : null}

      <div className="flex gap-2">
        <Input
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') void handleSend();
          }}
          disabled={!sessionId}
        />
        <Button onClick={() => void handleSend()} disabled={!sessionId}>
          发送
        </Button>
      </div>

      <pre
        ref={outputRef}
        className="border-line bg-surface flex-1 overflow-auto rounded border p-3 font-mono text-xs whitespace-pre-wrap"
      >
        {output || '（输出区）'}
      </pre>
    </div>
  );
}
