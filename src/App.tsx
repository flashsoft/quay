import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { parse } from 'smol-toml';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import {
  FileText,
  Logs,
  MoonStar,
  Play,
  Power,
  RotateCcw,
  Settings2,
  SunMedium,
  Waypoints,
} from 'lucide-react';

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card';
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty';
import { Field, FieldContent, FieldDescription, FieldError, FieldGroup, FieldTitle } from '@/components/ui/field';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Separator } from '@/components/ui/separator';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { cn } from '@/lib/utils';

type ThemeMode = 'light' | 'dark';
type AppView = 'main' | 'settings' | 'tray';
type ServiceState = 'running' | 'stopped' | 'error';

interface AppStatePayload {
  config: string;
  status: ServiceState;
  autostartEnabled: boolean;
  autoStartFrpcOnLaunch: boolean;
  configPath: string;
  logPath: string;
}

const APP_NAME = 'Quay';
const MOCK_CONFIG = `[common]\nserverAddr = "example.com"\nserverPort = 7000\nauth.method = "token"
auth.token = "replace-me"\n\n[[proxies]]\nname = "ssh"\ntype = "tcp"\nlocalIP = "127.0.0.1"\nlocalPort = 8080\nremotePort = 6000\n`;
const THEME_STORAGE_KEY = 'quay-theme';

const mockStore: AppStatePayload & { logs: string[] } = {
  config: MOCK_CONFIG,
  status: 'stopped',
  autostartEnabled: true,
  autoStartFrpcOnLaunch: true,
  configPath: '~/.config/quay/frpc.toml',
  logPath: '~/Library/Logs/Quay/frpc.log',
  logs: ['12:04:11 [I] Quay mock mode ready.'],
};

function isTauriEnvironment() {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function getInitialView(): AppView {
  if (typeof window === 'undefined') return 'main';
  const value = new URLSearchParams(window.location.search).get('view');
  return value === 'settings' || value === 'tray' ? value : 'main';
}

function getInitialTheme(): ThemeMode {
  if (typeof window === 'undefined') return 'dark';
  const byQuery = new URLSearchParams(window.location.search).get('theme');
  if (byQuery === 'light' || byQuery === 'dark') return byQuery;
  const saved = window.localStorage.getItem(THEME_STORAGE_KEY);
  if (saved === 'light' || saved === 'dark') return saved;
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function validateToml(value: string) {
  try {
    parse(value);
    return { ok: true as const, message: '配置格式正常。' };
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    return { ok: false as const, message: text };
  }
}

function pushMockLog(message: string) {
  const timestamp = new Date().toLocaleTimeString('zh-CN', { hour12: false });
  mockStore.logs.push(`${timestamp} ${message}`);
  mockStore.logs = mockStore.logs.slice(-120);
}

async function backendInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauriEnvironment()) {
    return invoke<T>(command, args);
  }

  switch (command) {
    case 'get_app_state':
      return structuredClone({
        config: mockStore.config,
        status: mockStore.status,
        autostartEnabled: mockStore.autostartEnabled,
        autoStartFrpcOnLaunch: mockStore.autoStartFrpcOnLaunch,
        configPath: mockStore.configPath,
        logPath: mockStore.logPath,
      }) as T;
    case 'read_logs':
      return structuredClone(mockStore.logs) as T;
    case 'save_config':
      mockStore.config = String(args?.content ?? '');
      pushMockLog('[I] config saved');
      return undefined as T;
    case 'start_frpc':
      mockStore.status = 'running';
      pushMockLog('[I] frpc started in mock mode');
      return undefined as T;
    case 'stop_frpc':
      mockStore.status = 'stopped';
      pushMockLog('[I] frpc stopped in mock mode');
      return undefined as T;
    case 'restart_frpc':
      mockStore.status = 'running';
      pushMockLog('[I] frpc restarted in mock mode');
      return undefined as T;
    case 'set_autostart':
      mockStore.autostartEnabled = Boolean(args?.enabled);
      return undefined as T;
    case 'set_auto_start_frpc_on_launch':
      mockStore.autoStartFrpcOnLaunch = Boolean(args?.enabled);
      return undefined as T;
    case 'open_settings_window':
    case 'open_main_window':
    case 'hide_current_window':
      return undefined as T;
    default:
      throw new Error(`Unsupported command in mock mode: ${command}`);
  }
}

function AppSurface({
  children,
  className,
}: React.ComponentProps<'section'>) {
  return (
    <section data-slot="app-surface" className={cn('mx-auto flex w-full flex-col gap-4', className)}>
      {children}
    </section>
  );
}

function WindowTitleBar({
  title,
  description,
  actions,
  className,
}: React.ComponentProps<'div'> & {
  title: string;
  description: string;
  actions?: React.ReactNode;
}) {
  return (
    <div
      data-slot="window-titlebar"
      className={cn('flex min-h-12 select-none items-center justify-between gap-4 pb-2', className)}
    >
      <div className="grid min-w-0 gap-1">
        <h1 className="text-lg font-semibold tracking-tight">{title}</h1>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </div>
  );
}

function statusMeta(status: ServiceState) {
  if (status === 'running') {
    return {
      label: 'Running',
      summary: 'frpc 正在运行，可以随时停止或重载配置。',
      tone: 'running' as const,
    };
  }

  if (status === 'error') {
    return {
      label: 'Error',
      summary: 'frpc 已异常退出，请检查配置或最近日志。',
      tone: 'error' as const,
    };
  }

  return {
    label: 'Stopped',
    summary: 'frpc 当前未启动。',
    tone: 'stopped' as const,
  };
}

function StatusBadge({
  tone,
  children,
}: {
  tone: 'running' | 'error' | 'stopped';
  children: React.ReactNode;
}) {
  return (
    <Badge
      variant="outline"
      data-slot="status-badge"
      data-status={tone}
      className="status-badge h-8 gap-2 rounded-md px-3 font-medium"
    >
      <span data-slot="status-badge-dot" className="status-badge-dot size-2 rounded-full" />
      {children}
    </Badge>
  );
}

function ThemeToggle({
  value,
  onChange,
  iconOnly = false,
}: {
  value: ThemeMode;
  onChange: (value: ThemeMode) => void;
  iconOnly?: boolean;
}) {
  return (
    <ToggleGroup
      type="single"
      variant="outline"
      size="sm"
      value={value}
      className={cn(iconOnly && 'gap-0')}
      onValueChange={(nextValue) => {
        if (nextValue === 'light' || nextValue === 'dark') onChange(nextValue);
      }}
    >
      <ToggleGroupItem value="light" aria-label="使用浅色模式" className={cn(iconOnly && 'rounded-r-none')}>
        <SunMedium data-icon="inline-start" />
        {iconOnly ? <span className="sr-only">使用浅色模式</span> : 'Light'}
      </ToggleGroupItem>
      <ToggleGroupItem value="dark" aria-label="使用深色模式" className={cn(iconOnly && 'rounded-l-none border-l-0')}>
        <MoonStar data-icon="inline-start" />
        {iconOnly ? <span className="sr-only">使用深色模式</span> : 'Dark'}
      </ToggleGroupItem>
    </ToggleGroup>
  );
}

function renderTomlLine(line: string, index: number) {
  const trimmed = line.trim();
  if (!trimmed) return <div key={index} className="min-h-6">&nbsp;</div>;
  if (trimmed.startsWith('#')) return <div key={index}><span className="token-comment">{line}</span></div>;
  if (/^\[\[?.*\]\]?$/.test(trimmed)) return <div key={index}><span className="token-section">{line}</span></div>;

  const match = line.match(/^(\s*)([A-Za-z0-9_.-]+)(\s*=\s*)(.*)$/);
  if (!match) return <div key={index}><span className="token-plain">{line}</span></div>;

  const [, indent, key, operator, rawValue] = match;
  const value = rawValue.trim();
  let tokenClass = 'token-plain';
  if (/^".*"$/.test(value) || /^'.*'$/.test(value)) tokenClass = 'token-string';
  else if (/^(true|false)$/.test(value)) tokenClass = 'token-boolean';
  else if (/^-?\d+(\.\d+)?$/.test(value)) tokenClass = 'token-number';

  return (
    <div key={index}>
      <span className="token-plain">{indent}</span>
      <span className="token-key">{key}</span>
      <span className="token-operator">{operator}</span>
      <span className={tokenClass}>{rawValue}</span>
    </div>
  );
}

function TomlEditor({
  value,
  onChange,
  invalid,
  className,
  shortcutNonce,
}: {
  value: string;
  onChange: (next: string) => void;
  invalid?: boolean;
  className?: string;
  shortcutNonce?: number;
}) {
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const highlightRef = useRef<HTMLPreElement | null>(null);

  const syncScroll = useCallback(() => {
    if (!textareaRef.current || !highlightRef.current) return;
    highlightRef.current.scrollTop = textareaRef.current.scrollTop;
    highlightRef.current.scrollLeft = textareaRef.current.scrollLeft;
  }, []);

  const selectAll = useCallback(() => {
    if (!textareaRef.current) return;
    textareaRef.current.focus();
    textareaRef.current.setSelectionRange(0, textareaRef.current.value.length);
  }, []);

  useEffect(() => {
    if (!shortcutNonce) return;
    selectAll();
  }, [selectAll, shortcutNonce]);

  return (
    <div
      data-slot="toml-editor"
      className={cn('relative h-96 min-h-0 overflow-hidden rounded-md border bg-background transition-colors focus-within:border-ring focus-within:ring-1 focus-within:ring-ring', className)}
    >
      <pre
        ref={highlightRef}
        data-slot="toml-editor-highlight"
        aria-hidden
        className="toml-highlight pointer-events-none h-full overflow-auto px-4 py-2 font-mono text-sm leading-6"
      >
        {value.split('\n').map(renderTomlLine)}
        {value.endsWith('\n') ? <div className="min-h-6">&nbsp;</div> : null}
      </pre>
      <Textarea
        ref={textareaRef}
        data-slot="toml-editor-input"
        aria-label="frpc.toml 配置编辑器"
        aria-invalid={invalid}
        spellCheck={false}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a') {
            event.preventDefault();
            selectAll();
          }
        }}
        onScroll={syncScroll}
        className="absolute inset-0 h-full resize-none overflow-auto border-0 bg-transparent px-4 py-2 font-mono text-sm leading-6 text-transparent shadow-none selection:bg-accent/40 focus-visible:ring-0"
        style={{ caretColor: 'hsl(var(--foreground))', WebkitTextFillColor: 'transparent' }}
      />
    </div>
  );
}

export default function App() {
  const [theme, setTheme] = useState<ThemeMode>(getInitialTheme);
  const [view] = useState<AppView>(getInitialView);

  useEffect(() => {
    if (typeof window !== 'undefined') {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    }
  }, [theme]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== 'w') return;

      event.preventDefault();
      void backendInvoke('hide_current_window');
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  return (
    <div className={theme}>
      {view === 'settings' ? (
        <SettingsWindow />
      ) : view === 'tray' ? (
        <TrayMenu />
      ) : (
        <MainWindow theme={theme} setTheme={setTheme} />
      )}
    </div>
  );
}

function MainWindow({
  theme,
  setTheme,
}: {
  theme: ThemeMode;
  setTheme: (value: ThemeMode) => void;
}) {
  const [appState, setAppState] = useState<AppStatePayload | null>(null);
  const [configDraft, setConfigDraft] = useState('');
  const [savedConfig, setSavedConfig] = useState('');
  const [logs, setLogs] = useState<string[]>([]);
  const [busyAction, setBusyAction] = useState<'save' | 'start' | 'stop' | 'restart' | 'reload' | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [editorShortcutNonce, setEditorShortcutNonce] = useState(0);

  const validation = useMemo(() => validateToml(configDraft), [configDraft]);
  const isDirty = configDraft !== savedConfig;
  const meta = statusMeta(appState?.status ?? 'stopped');
  const configStatus = !validation.ok
    ? { label: '格式错误', variant: 'destructive' as const }
    : isDirty
      ? { label: '未保存', variant: 'secondary' as const }
      : { label: '已保存', variant: 'outline' as const };

  const refreshState = useCallback(async () => {
    try {
      const nextState = await backendInvoke<AppStatePayload>('get_app_state');
      setAppState(nextState);
      setSavedConfig(nextState.config);
      setConfigDraft((current) => (current && current !== savedConfig ? current : nextState.config));
      setErrorMessage(null);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    }
  }, [savedConfig]);

  const refreshLogs = useCallback(async () => {
    try {
      const nextLogs = await backendInvoke<string[]>('read_logs');
      setLogs(nextLogs);
    } catch (error) {
      setErrorMessage(error instanceof Error ? error.message : String(error));
    }
  }, []);

  useEffect(() => {
    void refreshState();
    void refreshLogs();
  }, [refreshLogs, refreshState]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void refreshState();
      void refreshLogs();
    }, 1500);
    return () => window.clearInterval(timer);
  }, [refreshLogs, refreshState]);

  useEffect(() => {
    if (!isTauriEnvironment()) return;

    const unlistenHandles: Array<() => void> = [];
    void listen<ServiceState>('frpc-status', (event) => {
      setAppState((current) =>
        current
          ? {
              ...current,
              status: event.payload,
            }
          : current,
      );
      void refreshLogs();
      void refreshState();
    }).then((dispose) => {
      unlistenHandles.push(dispose);
    });

    void listen<string>('editor-shortcut', (event) => {
      if (event.payload === 'select-all') {
        setEditorShortcutNonce(Date.now());
      }
    }).then((dispose) => {
      unlistenHandles.push(dispose);
    });

    return () => {
      unlistenHandles.forEach((dispose) => dispose());
    };
  }, [refreshLogs, refreshState]);

  const runAction = useCallback(
    async (kind: 'save' | 'start' | 'stop' | 'restart' | 'reload', task: () => Promise<void>) => {
      setBusyAction(kind);
      try {
        await task();
        await refreshState();
        await refreshLogs();
        setErrorMessage(null);
      } catch (error) {
        setErrorMessage(error instanceof Error ? error.message : String(error));
      } finally {
        setBusyAction(null);
      }
    },
    [refreshLogs, refreshState],
  );

  const saveConfig = useCallback(async () => {
    if (!validation.ok) {
      setErrorMessage('当前 TOML 格式有误，无法保存。');
      return;
    }
    await runAction('save', async () => {
      await backendInvoke('save_config', { content: configDraft });
      setSavedConfig(configDraft);
    });
  }, [configDraft, runAction, validation.ok]);

  const saveAndReload = useCallback(async () => {
    if (!validation.ok) {
      setErrorMessage('当前 TOML 格式有误，无法重载。');
      return;
    }
    await runAction('reload', async () => {
      await backendInvoke('save_config', { content: configDraft });
      setSavedConfig(configDraft);
      await backendInvoke('restart_frpc');
    });
  }, [configDraft, runAction, validation.ok]);

  const serviceButtons =
    appState?.status === 'running' ? (
      <>
        <Button variant="outline" size="sm" disabled={busyAction !== null} onClick={() => void runAction('stop', () => backendInvoke('stop_frpc'))}>
          <Power data-icon="inline-start" />
          停止
        </Button>
        <Button variant="outline" size="sm" disabled={busyAction !== null || !validation.ok} onClick={() => void saveAndReload()}>
          <RotateCcw data-icon="inline-start" />
          重载配置
        </Button>
      </>
    ) : appState?.status === 'error' ? (
      <Button size="sm" disabled={busyAction !== null || !validation.ok} onClick={() => void runAction('restart', async () => {
        if (isDirty) {
          await backendInvoke('save_config', { content: configDraft });
          setSavedConfig(configDraft);
        }
        await backendInvoke('restart_frpc');
      })}>
        <RotateCcw data-icon="inline-start" />
        重新启动
      </Button>
    ) : (
      <Button size="sm" disabled={busyAction !== null || !validation.ok} onClick={() => void runAction('start', async () => {
        if (isDirty) {
          await backendInvoke('save_config', { content: configDraft });
          setSavedConfig(configDraft);
        }
        await backendInvoke('start_frpc');
      })}>
        <Play data-icon="inline-start" />
        启动
      </Button>
    );

  return (
    <main className="min-h-screen bg-background px-6 py-6 text-foreground">
      <AppSurface className="max-w-4xl">
        <WindowTitleBar
          title={APP_NAME}
          description="一个用于管理 frpc 的轻量桌面端。"
          actions={
            <>
              <ThemeToggle value={theme} onChange={setTheme} iconOnly />
              <Button variant="ghost" size="sm" onClick={() => (isTauriEnvironment() ? void backendInvoke('open_settings_window') : undefined)}>
                <Settings2 data-icon="inline-start" />
                设置
              </Button>
            </>
          }
        />
        <div className="flex flex-col gap-4">
          <Card>
            <CardHeader className="p-4">
              <div className="flex flex-wrap items-center justify-between gap-4">
                <div className="grid min-w-0 gap-1">
                  <CardTitle className="flex items-center gap-2 text-sm font-semibold">
                    <Waypoints data-icon="inline-start" />
                    服务控制
                  </CardTitle>
                  <CardDescription>{meta.summary}</CardDescription>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <StatusBadge tone={meta.tone}>{meta.label}</StatusBadge>
                  <ButtonGroup>{serviceButtons}</ButtonGroup>
                </div>
              </div>
            </CardHeader>
            <CardContent className="px-4 pb-4 pt-0">
              <div className="flex min-w-0 flex-wrap items-center gap-4 text-xs text-muted-foreground">
                <span className="min-w-0 break-all" title={appState?.configPath}>配置文件：{appState?.configPath ?? '加载中...'}</span>
                <Separator orientation="vertical" className="h-3" />
                <span className="min-w-0 break-all" title={appState?.logPath}>日志文件：{appState?.logPath ?? '加载中...'}</span>
              </div>
            </CardContent>
          </Card>

          <div className="grid gap-4 [grid-template-columns:repeat(auto-fit,minmax(min(100%,24rem),1fr))]">
            <Card className="flex h-[28rem] min-w-0 flex-col">
              <CardHeader className="flex flex-row items-start justify-between gap-4 p-4">
                <div className="grid min-w-0 gap-1">
                  <CardTitle className="flex items-center gap-2 text-sm font-semibold">
                    <FileText data-icon="inline-start" />
                    配置文件
                  </CardTitle>
                  <CardDescription>编辑 frpc.toml。运行中保存后可立即重载。</CardDescription>
                </div>
                <Badge variant={configStatus.variant} className="shrink-0">{configStatus.label}</Badge>
              </CardHeader>
              <CardContent className="flex min-h-0 flex-1 px-4 pb-4 pt-0">
                <Field data-invalid={!validation.ok} className="min-h-0 flex-1 gap-2">
                  <FieldContent className="sr-only">
                    <FieldTitle>frpc.toml 配置内容</FieldTitle>
                    <FieldDescription>修改 frpc.toml 并保存到本地配置文件。</FieldDescription>
                  </FieldContent>
                  <TomlEditor
                    className="min-h-0 flex-1"
                    value={configDraft}
                    onChange={setConfigDraft}
                    invalid={!validation.ok}
                    shortcutNonce={editorShortcutNonce}
                  />
                  <FieldError>{validation.ok ? null : `TOML 格式错误：${validation.message}`}</FieldError>
                </Field>
              </CardContent>
              <CardFooter className="flex items-center justify-end gap-2 px-4 pb-4 pt-0">
                <ButtonGroup>
                  <Button variant="outline" size="sm" disabled={busyAction !== null || !validation.ok || !isDirty} onClick={() => void saveConfig()}>
                    保存
                  </Button>
                  {appState?.status === 'running' ? (
                    <Button size="sm" disabled={busyAction !== null || !validation.ok} onClick={() => void saveAndReload()}>
                      保存并重载
                    </Button>
                  ) : null}
                </ButtonGroup>
              </CardFooter>
            </Card>

            <Card className="flex h-[28rem] min-w-0 flex-col">
              <CardHeader className="p-4">
                <div className="flex items-center justify-between gap-4">
                  <div className="grid min-w-0 gap-1">
                    <CardTitle className="flex items-center gap-2 text-sm font-semibold">
                      <Logs data-icon="inline-start" />
                      最近日志
                    </CardTitle>
                    <CardDescription>自动刷新最近 120 行输出。</CardDescription>
                  </div>
                  <Button variant="ghost" size="sm" className="h-8" disabled={busyAction !== null} onClick={() => void refreshLogs()}>
                    刷新
                  </Button>
                </div>
              </CardHeader>
              <CardContent className="flex min-h-0 flex-1 px-4 pb-4 pt-0">
                <ScrollArea className="min-h-0 flex-1 rounded-md border bg-muted/30">
                  <div className="flex flex-col gap-1 p-4 font-mono text-xs leading-5 text-muted-foreground">
                    {logs.length ? (
                      logs.map((line, index) => <div key={`${index}-${line}`}>{line}</div>)
                    ) : (
                      <Empty className="min-h-40 border-0 p-4">
                        <EmptyHeader>
                          <EmptyMedia variant="icon">
                            <Logs />
                          </EmptyMedia>
                          <EmptyTitle className="text-sm">暂无日志输出</EmptyTitle>
                          <EmptyDescription>启动 frpc 后，这里会显示最近 120 行输出。</EmptyDescription>
                        </EmptyHeader>
                      </Empty>
                    )}
                  </div>
                </ScrollArea>
              </CardContent>
            </Card>
          </div>

          {errorMessage ? (
            <Alert>
              <AlertTitle>操作失败</AlertTitle>
              <AlertDescription>{errorMessage}</AlertDescription>
            </Alert>
          ) : null}
        </div>
      </AppSurface>

      {!isTauriEnvironment() ? (
        <div className="mx-auto mt-4 max-w-4xl text-center text-xs text-muted-foreground">
          当前是浏览器预览模式，服务控制与日志使用 mock 数据。
        </div>
      ) : null}

    </main>
  );
}

function SettingsWindow() {
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [autoConnectEnabled, setAutoConnectEnabled] = useState(true);
  const [saving, setSaving] = useState<'autostart' | 'connect' | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const state = await backendInvoke<AppStatePayload>('get_app_state');
      setAutostartEnabled(state.autostartEnabled);
      setAutoConnectEnabled(state.autoStartFrpcOnLaunch);
      setMessage(null);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const toggleAutostart = useCallback(async (checked: boolean) => {
    setSaving('autostart');
    try {
      await backendInvoke('set_autostart', { enabled: checked });
      setAutostartEnabled(checked);
      setMessage(null);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(null);
    }
  }, []);

  const toggleAutoConnect = useCallback(async (checked: boolean) => {
    setSaving('connect');
    try {
      await backendInvoke('set_auto_start_frpc_on_launch', { enabled: checked });
      setAutoConnectEnabled(checked);
      setMessage(null);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setSaving(null);
    }
  }, []);

  return (
    <main className="min-h-screen bg-background px-5 py-5 text-foreground">
      <AppSurface className="max-w-lg gap-3">
        <WindowTitleBar
          title="设置"
          description="变更会立即保存，无需手动保存。"
          actions={<Badge variant="secondary">自动保存</Badge>}
        />
        <div className="flex flex-col gap-3">
          <Card>
            <CardHeader className="p-4 pb-3">
              <CardTitle className="flex items-center gap-2 text-sm font-semibold">
                <Settings2 data-icon="inline-start" />
                启动行为
              </CardTitle>
              <CardDescription>控制 Quay 何时打开，以及是否自动启动 frpc。</CardDescription>
            </CardHeader>
            <CardContent className="px-4 pb-4 pt-0">
              <FieldGroup className="gap-3">
                <Field orientation="horizontal" data-disabled={saving !== null}>
                  <FieldContent>
                    <FieldTitle>登录后打开 Quay</FieldTitle>
                    <FieldDescription>macOS 登录后自动打开桌面端。</FieldDescription>
                  </FieldContent>
                  <Switch
                    aria-label="登录后打开 Quay"
                    checked={autostartEnabled}
                    disabled={saving !== null}
                    onCheckedChange={(checked) => void toggleAutostart(checked)}
                  />
                </Field>
                <Separator />
                <Field orientation="horizontal" data-disabled={saving !== null}>
                  <FieldContent>
                    <FieldTitle>打开后启动 frpc</FieldTitle>
                    <FieldDescription>Quay 启动后按当前配置连接 frpc。</FieldDescription>
                  </FieldContent>
                  <Switch
                    aria-label="打开后启动 frpc"
                    checked={autoConnectEnabled}
                    disabled={saving !== null}
                    onCheckedChange={(checked) => void toggleAutoConnect(checked)}
                  />
                </Field>
              </FieldGroup>
            </CardContent>
          </Card>

          {message ? (
            <Alert>
              <AlertTitle>设置失败</AlertTitle>
              <AlertDescription>{message}</AlertDescription>
            </Alert>
          ) : null}
        </div>
      </AppSurface>
    </main>
  );
}

function TrayMenu() {
  return (
    <main className="min-h-screen bg-background px-6 py-6 text-foreground">
      <AppSurface className="max-w-sm">
        <Card data-slot="tray-menu-preview">
          <CardContent className="flex flex-col gap-2 p-2">
            <Button variant="ghost" className="justify-start" disabled>打开主窗口</Button>
            <Button variant="ghost" className="justify-start" disabled>打开设置</Button>
            <Separator />
            <Button variant="ghost" className="justify-start" disabled>启动 frpc</Button>
            <Button variant="ghost" className="justify-start" disabled>退出</Button>
          </CardContent>
        </Card>
      </AppSurface>
    </main>
  );
}
