/**
 * 首次初始化引导页面
 */
import { useState, useEffect } from 'react';
import { Form, Input, Button, Card, Steps, Select, message, Typography, Divider } from 'antd';
import { DatabaseOutlined, UserOutlined, LockOutlined, CheckCircleOutlined } from '@ant-design/icons';
import { useNavigate } from 'react-router-dom';
import { authApi } from '../api/client';

const { Title, Paragraph } = Typography;
const { Option } = Select;

interface SetupFormData {
  // 数据库配置
  dbType: 'mysql';
  dbHost: string;
  dbPort: number;
  dbUser: string;
  dbPassword: string;
  dbDatabase: string;
  // 管理员账号
  adminUsername: string;
  adminPassword: string;
  adminConfirmPassword: string;
}

export default function SetupPage() {
  const [current, setCurrent] = useState(0);
  const [loading, setLoading] = useState(false);
  const [initialized, setInitialized] = useState(false);
  const [checking, setChecking] = useState(true);
  const [form] = Form.useForm<SetupFormData>();
  const navigate = useNavigate();

  // 检查是否已初始化
  useEffect(() => {
    const checkSetup = async () => {
      try {
        const status = await authApi.getSetupStatus();
        if (status.initialized) {
          setInitialized(true);
          navigate('/login');
        }
      } catch {
        // 未初始化，继续
      } finally {
        setChecking(false);
      }
    };
    checkSetup();
  }, [navigate]);

  const handleDbTest = async () => {
    message.info('数据库连接测试功能开发中...');
  };

  const onFinish = async () => {
    const values = form.getFieldsValue();

    if (values.adminPassword !== values.adminConfirmPassword) {
      message.error('两次密码输入不一致');
      return;
    }

    if (values.adminPassword.length < 6) {
      message.error('密码长度至少为 6 位');
      return;
    }

    setLoading(true);
    try {
      await authApi.setup({
        database: {
          type: 'mysql',
          host: values.dbHost,
          port: values.dbPort,
          user: values.dbUser,
          password: values.dbPassword,
          database: values.dbDatabase,
        },
        admin: {
          username: values.adminUsername,
          password: values.adminPassword,
        },
      });
      setCurrent(2);
    } catch (error: unknown) {
      const err = error as { response?: { data?: { message?: string } } };
      message.error(err.response?.data?.message || '初始化失败');
    } finally {
      setLoading(false);
    }
  };

  if (checking) {
    return (
      <div style={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}>
        加载中...
      </div>
    );
  }

  if (initialized) {
    return null; // 已经在 useEffect 中重定向了
  }

  const steps = [
    { title: '数据库配置', icon: <DatabaseOutlined /> },
    { title: '创建管理员', icon: <UserOutlined /> },
    { title: '完成', icon: <CheckCircleOutlined /> },
  ];

  return (
    <div style={{
      minHeight: '100vh',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      background: 'linear-gradient(135deg, #667eea 0%, #764ba2 100%)',
      padding: '24px',
    }}>
      <Card style={{ width: 600, boxShadow: '0 14px 40px rgba(0,0,0,0.2)' }}>
        <div style={{ textAlign: 'center', marginBottom: 24 }}>
          <Title level={2}>Shadow 初始化</Title>
          <Paragraph type="secondary">
            首次使用需要配置数据库和管理员账号
          </Paragraph>
        </div>

        <Steps current={current} items={steps} style={{ marginBottom: 32 }} />

        {current === 0 && (
          <>
            <Form form={form} layout="vertical" initialValues={{ dbType: 'mysql', dbPort: 3306 }}>
              <Form.Item label="数据库类型" name="dbType">
                <Select>
                  <Option value="mysql">MySQL</Option>
                </Select>
              </Form.Item>

              <Form.Item label="主机地址" name="dbHost" rules={[{ required: true, message: '请输入主机地址' }]}>
                <Input placeholder="localhost" />
              </Form.Item>

              <Form.Item label="端口" name="dbPort" rules={[{ required: true, message: '请输入端口' }]}>
                <Input type="number" placeholder="3306" />
              </Form.Item>

              <Form.Item label="用户名" name="dbUser" rules={[{ required: true, message: '请输入用户名' }]}>
                <Input placeholder="root" />
              </Form.Item>

              <Form.Item label="密码" name="dbPassword">
                <Input.Password placeholder="留空则无密码" />
              </Form.Item>

              <Form.Item label="数据库名" name="dbDatabase" rules={[{ required: true, message: '请输入数据库名' }]}>
                <Input placeholder="shadow" />
              </Form.Item>

              <Form.Item>
                <Button type="default" onClick={handleDbTest} style={{ marginRight: 8 }}>
                  测试连接
                </Button>
                <Button type="primary" onClick={() => setCurrent(1)}>
                  下一步
                </Button>
              </Form.Item>
            </Form>
          </>
        )}

        {current === 1 && (
          <>
            <Divider>管理员账号</Divider>
            <Form form={form} layout="vertical">
              <Form.Item
                label="管理员用户名"
                name="adminUsername"
                rules={[{ required: true, message: '请输入管理员用户名' }]}
              >
                <Input prefix={<UserOutlined />} placeholder="admin" />
              </Form.Item>

              <Form.Item
                label="密码"
                name="adminPassword"
                rules={[
                  { required: true, message: '请输入密码' },
                  { min: 6, message: '密码长度至少为 6 位' },
                ]}
              >
                <Input.Password prefix={<LockOutlined />} placeholder="至少 6 位" />
              </Form.Item>

              <Form.Item
                label="确认密码"
                name="adminConfirmPassword"
                rules={[
                  { required: true, message: '请确认密码' },
                  ({ getFieldValue }) => ({
                    validator(_, value) {
                      if (!value || getFieldValue('adminPassword') === value) {
                        return Promise.resolve();
                      }
                      return Promise.reject(new Error('两次密码输入不一致'));
                    },
                  }),
                ]}
              >
                <Input.Password prefix={<LockOutlined />} placeholder="再次输入密码" />
              </Form.Item>

              <Form.Item>
                <Button onClick={() => setCurrent(0)} style={{ marginRight: 8 }}>
                  上一步
                </Button>
                <Button type="primary" loading={loading} onClick={onFinish}>
                  完成初始化
                </Button>
              </Form.Item>
            </Form>
          </>
        )}

        {current === 2 && (
          <>
            <div style={{ textAlign: 'center', padding: '24px 0' }}>
              <CheckCircleOutlined style={{ fontSize: 64, color: '#52c41a', marginBottom: 16 }} />
              <Title level={3}>初始化完成</Title>
              <Paragraph>Shadow 管理面板已配置完成，请使用管理员账号登录。</Paragraph>
              <Button type="primary" size="large" onClick={() => navigate('/login')}>
                前往登录
              </Button>
            </div>
          </>
        )}
      </Card>
    </div>
  );
}
