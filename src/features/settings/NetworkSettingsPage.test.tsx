import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, expect, it, vi } from 'vitest';
import { NetworkSettingsPage } from '@/features/settings/NetworkSettingsPage';

const { all, write } = vi.hoisted(() => ({ all: vi.fn(), write: vi.fn(async () => undefined) }));
vi.mock('@/lib/ipc', () => ({
  settingsAll: all,
  settingsSet: write,
  gpgListSecretKeys: vi.fn(async () => []),
  gpgTestSign: vi.fn(),
  networkGitTest: vi.fn(),
  networkProxyTest: vi.fn(),
  sshTestConnection: vi.fn(),
}));
afterEach(cleanup);

it('异步读取的代理与绕过项显示在输入框，重新应用保持原值', async () => {
  const stored = {
    'network.proxyMode': JSON.stringify('manual'),
    'network.proxyUrl': JSON.stringify('http://127.0.0.1:7890'),
    'network.noProxy': JSON.stringify('localhost,.internal'),
  };
  all.mockImplementation(async () => stored);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <NetworkSettingsPage />
    </QueryClientProvider>,
  );
  await screen.findByDisplayValue('http://127.0.0.1:7890');
  expect(screen.getByDisplayValue('localhost,.internal')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /应用/ }));
  await waitFor(() =>
    expect(write).toHaveBeenCalledWith(
      'global',
      'network.proxyUrl',
      stored['network.proxyUrl'],
      undefined,
    ),
  );
  client.clear();
});
