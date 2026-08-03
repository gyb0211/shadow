/**
 * 配置管理页面 -- 限高滚动 + 复制按钮
 */
import { Card, Button, message, Alert, Tooltip } from 'antd';
import { ReloadOutlined, CopyOutlined } from '@ant-design/icons';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { configApi } from '../api/client';
import PageHeader from '../components/PageHeader';

export default function ConfigPage() {
  const { data: config, isLoading, error } = useQuery({
    queryKey: ['config'],
    queryFn: configApi.get,
  });

  const queryClient = useQueryClient();

  const handleReload = () => {
    queryClient.invalidateQueries({ queryKey: ['config'] });
    message.success('配置已刷新');
  };

  const handleCopy = () => {
    navigator.clipboard.writeText(JSON.stringify(config, null, 2));
    message.success('已复制到剪贴板');
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <PageHeader
        title="配置管理"
        subtitle="查看当前运行配置"
        extra={
          <div style={{ display: 'flex', gap: 8 }}>
            <Tooltip title="复制 JSON">
              <Button icon={<CopyOutlined />} onClick={handleCopy} disabled={!config} />
            </Tooltip>
            <Button icon={<ReloadOutlined />} onClick={handleReload}>刷新</Button>
          </div>
        }
      />

      {error && (
        <Alert
          message="加载配置失败"
          description="无法从服务器获取配置信息"
          type="error"
          style={{ marginBottom: 12, borderRadius: 8, flexShrink: 0 }}
        />
      )}

      <Card
        className="fill-card no-pad"
        style={{ borderRadius: 12 }}
      >
        <pre style={{
          margin: 0,
          padding: 20,
          overflow: 'auto',
          height: '100%',
          fontSize: 13,
          fontFamily: 'monospace, monospace',
          color: 'rgba(255,255,255,0.75)',
          lineHeight: 1.6,
        }}>
          {isLoading ? '加载中...' : JSON.stringify(config, null, 2)}
        </pre>
      </Card>

      <Alert
        message="配置说明"
        description="当前配置为只读视图。编辑配置请直接修改 ~/.shadow/config.toml 文件后重启服务。"
        type="info"
        style={{ marginTop: 12, borderRadius: 8, flexShrink: 0 }}
      />
    </div>
  );
}
