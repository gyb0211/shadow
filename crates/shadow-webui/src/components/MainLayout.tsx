/**
 * 主布局组件
 */
import { Layout, Menu, Avatar, Dropdown, Typography, Space } from 'antd';
import {
  DashboardOutlined,
  RobotOutlined,
  WifiOutlined,
  CloudServerOutlined,
  ToolOutlined,
  SettingOutlined,
  FileTextOutlined,
  TeamOutlined,
  LogoutOutlined,
  UserOutlined,
} from '@ant-design/icons';
import { Outlet, useNavigate, useLocation } from 'react-router-dom';
import { useAuth } from '../contexts/AuthContext';

const { Header, Sider, Content } = Layout;
const { Text } = Typography;

const menuItems = [
  { key: '/', icon: <DashboardOutlined />, label: '概览' },
  { key: '/agents', icon: <RobotOutlined />, label: 'Agent' },
  { key: '/channels', icon: <WifiOutlined />, label: 'Channel' },
  { key: '/providers', icon: <CloudServerOutlined />, label: 'Provider' },
  { key: '/tools', icon: <ToolOutlined />, label: '工具' },
  { key: '/config', icon: <SettingOutlined />, label: '配置' },
  { key: '/logs', icon: <FileTextOutlined />, label: '日志' },
  { key: '/users', icon: <TeamOutlined />, label: '用户', adminOnly: true },
];

export default function MainLayout() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const isAdmin = user?.role === 'admin';

  const filteredItems = menuItems.filter((item) => !item.adminOnly || isAdmin);

  const userMenu = {
    items: [
      { key: 'profile', icon: <UserOutlined />, label: '个人信息' },
      { type: 'divider' as const },
      { key: 'logout', icon: <LogoutOutlined />, label: '退出登录', danger: true },
    ],
    onClick: ({ key }: { key: string }) => {
      if (key === 'logout') {
        logout();
        navigate('/login');
      }
    },
  };

  return (
    <Layout style={{ minHeight: '100vh' }}>
      <Sider theme="dark" width={220} style={{ position: 'fixed', left: 0, top: 0, bottom: 0 }}>
        <div style={{
          height: 64,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'white',
          fontSize: 18,
          fontWeight: 'bold',
          borderBottom: '1px solid rgba(255,255,255,0.1)',
        }}>
          Shadow
        </div>
        <Menu
          theme="dark"
          mode="inline"
          selectedKeys={[location.pathname]}
          items={filteredItems}
          onClick={({ key }) => navigate(key)}
        />
      </Sider>

      <Layout style={{ marginLeft: 220 }}>
        <Header style={{
          background: '#fff',
          padding: '0 24px',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'flex-end',
          boxShadow: '0 2px 8px rgba(0,0,0,0.1)',
        }}>
          <Space>
            <Text type="secondary">{user?.username}</Text>
            <Text type="secondary" style={{ fontSize: 12 }}>({user?.role === 'admin' ? '管理员' : '查看者'})</Text>
            <Dropdown menu={userMenu} placement="bottomRight">
              <Avatar icon={<UserOutlined />} style={{ cursor: 'pointer' }} />
            </Dropdown>
          </Space>
        </Header>

        <Content style={{ margin: 24, padding: 24, background: '#fff', borderRadius: 8 }}>
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
