/**
 * 页面标题组件 -- 统一各页面顶部样式，flex-shrink:0 防止被压缩
 */
import { Typography, Space } from 'antd';
import type { ReactNode } from 'react';

const { Title } = Typography;

interface PageHeaderProps {
  title: string;
  subtitle?: string;
  extra?: ReactNode;
}

export default function PageHeader({ title, subtitle, extra }: PageHeaderProps) {
  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      marginBottom: 16,
      flexShrink: 0,
    }}>
      <div>
        <Title level={4} style={{
          color: 'rgba(255,255,255,0.85)',
          margin: 0,
          fontWeight: 600,
        }}>
          {title}
        </Title>
        {subtitle && (
          <Typography.Text style={{
            color: 'rgba(255,255,255,0.35)',
            fontSize: 13,
          }}>
            {subtitle}
          </Typography.Text>
        )}
      </div>
      {extra && <Space>{extra}</Space>}
    </div>
  );
}
