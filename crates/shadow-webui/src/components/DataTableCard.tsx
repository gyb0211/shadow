/**
 * 可复用的数据表格容器 -- 撑满父容器高度，内部滚动
 */
import { Card, Table } from 'antd';
import type { TableProps } from 'antd';
import type { ReactNode } from 'react';

interface DataTableCardProps<T> {
  loading?: boolean;
  dataSource: T[];
  columns: TableProps<T>['columns'];
  rowKey: string | ((record: T) => string | number);
  pageSize?: number;
  extra?: ReactNode;
  emptyText?: string;
}

export default function DataTableCard<T extends object>({
  loading,
  dataSource,
  columns,
  rowKey,
  pageSize = 15,
  extra,
  emptyText = '暂无数据',
}: DataTableCardProps<T>) {
  return (
    <Card
      className="fill-card no-pad"
      style={{ borderRadius: 12 }}
      extra={extra}
    >
      <Table<T>
        dataSource={dataSource}
        columns={columns}
        rowKey={rowKey}
        loading={loading}
        pagination={{
          pageSize,
          showSizeChanger: false,
          showTotal: (total) => `共 ${total} 条`,
          size: 'small',
        }}
        scroll={{ x: 'max-content' }}
        locale={{ emptyText }}
        size="middle"
      />
    </Card>
  );
}
