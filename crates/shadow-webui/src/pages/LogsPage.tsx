/**
 * 日志查看页面
 */
import { useState } from 'react';
import { Card, Select, Button, Tag, Space, Empty } from 'antd';
import { ReloadOutlined } from '@ant-design/icons';
import { useQuery } from '@tanstack/react-query';
import { logsApi } from '../api/client';
import PageHeader from '../components/PageHeader';

const levelColors: Record<string, string> = {
  ERROR: 'red',
  WARN: 'orange',
  INFO: 'blue',
  DEBUG: 'default',
  TRACE: 'default',
};

export default function LogsPage() {
  const [level, setLevel] = useState<string>('INFO');
  const [limit, setLimit] = useState<number>(100);

  const { data: logs, isLoading, refetch } = useQuery({
    queryKey: ['logs', level, limit],
    queryFn: () => logsApi.list({ level, limit }),
  });

  const columns = [
    {
      title: '时间',
      dataIndex: 'timestamp',
      key: 'timestamp',
      width: 180,
      render: (v: string) => <span style={{ color: 'rgba(255,255,255,0.45)', fontFamily: 'monospace, monospace', fontSize: 12 }}>{v}</span>,
    },
    {
      title: '级别',
      dataIndex: 'level',
      key: 'level',
      width: 80,
      render: (v: string) => <Tag color={levelColors[v] || 'default'}>{v}</Tag>,
    },
    {
      title: '目标',
      dataIndex: 'target',
      key: 'target',
      width: 200,
      render: (v: string) => <span style={{ color: 'rgba(255,255,255,0.35)', fontFamily: 'monospace, monospace', fontSize: 12 }}>{v || '-'}</span>,
    },
    {
      title: '消息',
      dataIndex: 'message',
      key: 'message',
      render: (v: string) => <span style={{ color: 'rgba(255,255,255,0.75)' }}>{v}</span>,
    },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <PageHeader title="日志查看" subtitle="系统运行日志" />

      {/* 筛选栏 */}
      <Card
        style={{ borderRadius: 12, marginBottom: 12, flexShrink: 0 }}
        styles={{ body: { padding: '12px 16px' } }}
      >
        <Space>
          <Select value={level} onChange={setLevel} style={{ width: 120 }}>
            <Select.Option value="">全部</Select.Option>
            <Select.Option value="ERROR">ERROR</Select.Option>
            <Select.Option value="WARN">WARN</Select.Option>
            <Select.Option value="INFO">INFO</Select.Option>
            <Select.Option value="DEBUG">DEBUG</Select.Option>
          </Select>
          <Select value={limit} onChange={setLimit} style={{ width: 120 }}>
            <Select.Option value={50}>50 条</Select.Option>
            <Select.Option value={100}>100 条</Select.Option>
            <Select.Option value={200}>200 条</Select.Option>
            <Select.Option value={500}>500 条</Select.Option>
          </Select>
          <Button icon={<ReloadOutlined />} onClick={() => refetch()}>刷新</Button>
        </Space>
      </Card>

      {/* 日志表格 -- 撑满剩余空间 */}
      <Card
        className="fill-card no-pad"
        style={{ borderRadius: 12 }}
      >
        {logs && logs.length > 0 ? (
          <div style={{ flex: 1, overflow: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
            <thead style={{ position: 'sticky', top: 0, background: '#12121f', zIndex: 1 }}>
              <tr>
                {columns.map((c) => (
                  <th key={c.key} style={{
                    textAlign: 'left',
                    padding: '10px 16px',
                    color: 'rgba(255,255,255,0.45)',
                    fontSize: 12,
                    fontWeight: 500,
                    borderBottom: '1px solid rgba(255,255,255,0.06)',
                    whiteSpace: 'nowrap',
                  }}>
                    {c.title}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {logs.map((log, i) => (
                <tr key={i} style={{ borderBottom: '1px solid rgba(255,255,255,0.03)' }}>
                  <td style={{ padding: '8px 16px', color: 'rgba(255,255,255,0.45)', fontFamily: 'monospace, monospace', fontSize: 12, whiteSpace: 'nowrap' }}>
                    {log.timestamp}
                  </td>
                  <td style={{ padding: '8px 16px' }}>
                    <Tag color={levelColors[log.level] || 'default'}>{log.level}</Tag>
                  </td>
                  <td style={{ padding: '8px 16px', color: 'rgba(255,255,255,0.35)', fontFamily: 'monospace, monospace', fontSize: 12, whiteSpace: 'nowrap' }}>
                    {log.target || '-'}
                  </td>
                  <td style={{ padding: '8px 16px', color: 'rgba(255,255,255,0.75)' }}>
                    {log.message}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          </div>
        ) : (
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%' }}>
            <Empty description={isLoading ? '加载中...' : '暂无日志'} />
          </div>
        )}
      </Card>
    </div>
  );
}
