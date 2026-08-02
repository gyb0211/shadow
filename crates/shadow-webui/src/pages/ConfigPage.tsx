/**
 * 配置管理页面
 */
import { Card, Button, message, Typography, Alert } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { configApi } from '../api/client';

const { Title } = Typography;

export default function ConfigPage() {
  const { data: config, isLoading, error } = useQuery({
    queryKey: ['config'],
    queryFn: configApi.get,
  });

  const queryClient = useQueryClient();

  const reloadMutation = useMutation({
    mutationFn: async () => {
      queryClient.invalidateQueries({ queryKey: ['config'] });
      message.success('配置已刷新');
    },
  });

  if (isLoading) {
    return <div>加载中...</div>;
  }

  return (
    <div>
      <Title level={2}>配置管理</Title>

      {error && (
        <Alert
          message="加载配置失败"
          description="无法从服务器获取配置信息"
          type="error"
          style={{ marginBottom: 16 }}
        />
      )}

      <Card
        extra={
          <Button icon={<ReloadOutlined />} onClick={() => reloadMutation.mutate()}>
            刷新
          </Button>
        }
      >
        <pre style={{ background: '#f5f5f5', padding: 16, borderRadius: 8, overflow: 'auto' }}>
          {JSON.stringify(config, null, 2)}
        </pre>
      </Card>

      <Alert
        message="配置说明"
        description="当前配置为只读视图。编辑配置请直接修改 ~/.shadow/config.toml 文件后重启服务。"
        type="info"
        style={{ marginTop: 16 }}
      />
    </div>
  );
}
