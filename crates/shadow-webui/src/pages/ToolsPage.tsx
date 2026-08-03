/**
 * 工具列表页面
 */
import { Tag } from 'antd';
import { useQuery } from '@tanstack/react-query';
import { toolsApi } from '../api/client';
import PageHeader from '../components/PageHeader';
import DataTableCard from '../components/DataTableCard';

export default function ToolsPage() {
  const { data: tools, isLoading } = useQuery({
    queryKey: ['tools'],
    queryFn: toolsApi.list,
  });

  const columns = [
    {
      title: '工具名称',
      dataIndex: 'name',
      key: 'name',
      width: 240,
      render: (v: string) => <Tag color="blue" style={{ fontFamily: 'monospace, monospace' }}>{v}</Tag>,
    },
    {
      title: '描述',
      dataIndex: 'description',
      key: 'description',
      render: (v: string) => <span style={{ color: 'rgba(255,255,255,0.65)' }}>{v}</span>,
    },
  ];

  return (
    <div style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
      <PageHeader title="工具列表" subtitle="已注册的可用工具" />
      <DataTableCard
        loading={isLoading}
        dataSource={tools || []}
        columns={columns}
        rowKey="name"
        pageSize={20}
        emptyText="暂无工具"
      />
    </div>
  );
}
