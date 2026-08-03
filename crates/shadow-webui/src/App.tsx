/**
 * Shadow WebUI 主应用
 * 深色主题 + 渐变设计语言
 */
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { ConfigProvider, theme } from 'antd';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import { AuthProvider, useAuth } from './contexts/AuthContext';

import LoginPage from './pages/LoginPage';
import SetupPage from './pages/SetupPage';
import DashboardPage from './pages/DashboardPage';
import AgentsPage from './pages/AgentsPage';
import ChannelsPage from './pages/ChannelsPage';
import ProvidersPage from './pages/ProvidersPage';
import ToolsPage from './pages/ToolsPage';
import ConfigPage from './pages/ConfigPage';
import LogsPage from './pages/LogsPage';
import UsersPage from './pages/UsersPage';

import MainLayout from './components/MainLayout';
import ProtectedRoute from './components/ProtectedRoute';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30000,
      retry: 1,
    },
  },
});

function AppRoutes() {
  const { isAuthenticated } = useAuth();

  return (
    <Routes>
      <Route path="/login" element={isAuthenticated ? <Navigate to="/" replace /> : <LoginPage />} />
      <Route path="/setup" element={<SetupPage />} />

      <Route
        path="/"
        element={
          <ProtectedRoute>
            <MainLayout />
          </ProtectedRoute>
        }
      >
        <Route index element={<DashboardPage />} />
        <Route path="agents" element={<AgentsPage />} />
        <Route path="channels" element={<ChannelsPage />} />
        <Route path="providers" element={<ProvidersPage />} />
        <Route path="tools" element={<ToolsPage />} />
        <Route path="config" element={<ConfigPage />} />
        <Route path="logs" element={<LogsPage />} />
        <Route path="users" element={<UsersPage />} />
      </Route>

      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

function App() {
  return (
    <ConfigProvider
      theme={{
        algorithm: theme.darkAlgorithm,
        token: {
          colorPrimary: '#6c5ce7',
          colorBgBase: '#0a0a14',
          colorBgContainer: '#12121f',
          colorBgElevated: '#1a1a2e',
          colorBorder: 'rgba(255,255,255,0.08)',
          colorBorderSecondary: 'rgba(255,255,255,0.06)',
          colorText: 'rgba(255,255,255,0.85)',
          colorTextSecondary: 'rgba(255,255,255,0.45)',
          borderRadius: 10,
          fontFamily: "'SF Pro Display', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
        },
        components: {
          Layout: {
            siderBg: '#0d0d18',
            headerBg: 'rgba(18,18,31,0.8)',
            headerPadding: '0 24px',
            headerHeight: 56,
          },
          Menu: {
            itemBg: 'transparent',
            itemColor: 'rgba(255,255,255,0.55)',
            itemHoverColor: 'rgba(255,255,255,0.85)',
            itemSelectedBg: 'rgba(108,92,231,0.15)',
            itemSelectedColor: '#a29bfe',
            itemBorderRadius: 8,
          },
          Card: {
            colorBgContainer: '#12121f',
            borderRadiusLG: 12,
          },
          Table: {
            colorBgContainer: '#12121f',
            rowHoverBg: 'rgba(255,255,255,0.03)',
          },
          Button: {
            borderRadius: 8,
          },
          Modal: {
            contentBg: '#12121f',
            headerBg: '#12121f',
          },
          Input: {
            colorBgContainer: '#0d0d18',
          },
          Select: {
            colorBgContainer: '#0d0d18',
          },
        },
      }}
    >
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <AuthProvider>
            <AppRoutes />
          </AuthProvider>
        </BrowserRouter>
      </QueryClientProvider>
    </ConfigProvider>
  );
}

export default App;
