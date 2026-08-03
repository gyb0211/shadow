/**
 * Channel 管理页面
 */
import { Tag } from 'antd';
import { useQuery } from '@tanstack/react-query';
import { channelsApi } from '../api/client';
import PageHeader from '../components/PageHeader';
import DataTableCard from '../components/DataTableCard';
import { JsonCell } from '../components/JsonCell';

export default function ChannelsPage() {
  const { data: channels, isLoading } = useQuery({
    queryKey: ['channels'],
    queryFn: channelsApi.list,
  });

  const columns = [
    {
      title: '类型',
      dataIndex: 'type',
      key: 'type',
      width: 120,
      render: (v: string) => <Tag color="purple">{v}</Tag>,
    },
    {
      title: '名称',
      dataIndex: 'alias',
      key: 'alias',
      width: 150,
      render: (v: string) => <span style={{ fontFamily: 'monospace, monospace', color: 'rgba(255,255,255,0.85)' }}>{v}</span>,
    },
    {
      title: '状态',
      dataIndex: 'enabled',
      key: 'enabled',
      width: 80,
      render: (v: boolean) => v ? <Tag color="success">启用</Tag> : <Tag>禁用</Tag>,
    },
    {
      title: '配置',
      dataIndex: 'config',
      key: 'config',
      render: (v: Record<string, unknown>) => <JsonCell data={v} />,
    },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <PageHeader title="Channel 管理" subtitle="消息渠道配置" />
      <DataTableCard
        loading={isLoading}
        dataSource={channels || []}
        columns={columns}
        rowKey="alias"
        emptyText="暂无 Channel"
      />
    </div>
  );
}
