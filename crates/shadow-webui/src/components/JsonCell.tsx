/**
 * JSON 预览弹窗 -- 点击 config 列查看完整 JSON
 */
import { Modal, Typography, Button, Tooltip } from 'antd';
import { CopyOutlined } from '@ant-design/icons';
import { useState } from 'react';
import { message } from 'antd';

const { Text } = Typography;

interface JsonPreviewProps {
  data: Record<string, unknown>;
  maxChars?: number;
}

/** 行内截断显示，点击弹出完整 JSON */
export function JsonCell({ data, maxChars = 60 }: JsonPreviewProps) {
  const [open, setOpen] = useState(false);
  const jsonStr = JSON.stringify(data);
  const truncated = jsonStr.length > maxChars
    ? jsonStr.slice(0, maxChars) + '...'
    : jsonStr;

  const handleCopy = () => {
    navigator.clipboard.writeText(JSON.stringify(data, null, 2));
    message.success('已复制到剪贴板');
  };

  return (
    <>
      <Tooltip title="点击查看完整配置">
        <Text
          onClick={() => setOpen(true)}
          style={{
            color: 'rgba(255,255,255,0.35)',
            fontFamily: 'monospace, monospace',
            fontSize: 12,
            cursor: 'pointer',
            display: 'block',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            maxWidth: 400,
          }}
        >
          {truncated}
        </Text>
      </Tooltip>
      <Modal
        title="配置详情"
        open={open}
        onCancel={() => setOpen(false)}
        footer={[
          <Button key="copy" icon={<CopyOutlined />} onClick={handleCopy}>复制</Button>,
          <Button key="close" type="primary" onClick={() => setOpen(false)}>关闭</Button>,
        ]}
        width={640}
      >
        <pre style={{
          background: 'rgba(0,0,0,0.3)',
          padding: 16,
          borderRadius: 8,
          overflow: 'auto',
          fontSize: 13,
          fontFamily: 'monospace, monospace',
          color: 'rgba(255,255,255,0.75)',
          lineHeight: 1.6,
          maxHeight: '60vh',
        }}>
          {JSON.stringify(data, null, 2)}
        </pre>
      </Modal>
    </>
  );
}
