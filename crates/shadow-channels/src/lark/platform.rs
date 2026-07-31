#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LarkPlatform {
    Lark,
    Feishu,
}

const FEISHU_BASE_URL: &str = "https://open.feishu.cn/open-apis";
const FEISHU_WS_BASE_URL: &str = "https://open.feishu.cn";
const LARK_BASE_URL: &str = "https://open.larksuite.com/open-apis";
const LARK_WS_BASE_URL: &str = "https://open.larksuite.com";

impl LarkPlatform {
    pub(super) fn proxy_service_key(self) -> &'static str {
        match self {
            Self::Lark => "channel.lark",
            Self::Feishu => "channel.feishu",
        }
    }

    pub(super) fn channel_name(self) -> &'static str {
        match self {
            LarkPlatform::Lark => "lark",
            LarkPlatform::Feishu => "feishu",
        }
    }

    pub(super) fn api_base(self) -> &'static str {
        match self {
            Self::Lark => LARK_BASE_URL,
            Self::Feishu => FEISHU_BASE_URL,
        }
    }

    pub(super) fn ws_base(self) -> &'static str {
        match self {
            Self::Lark => LARK_WS_BASE_URL,
            Self::Feishu => FEISHU_WS_BASE_URL,
        }
    }
    pub(super) fn locale_header(self) -> &'static str {
        match self {
            Self::Lark => "en",
            Self::Feishu => "zh",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lark_urls() {
        let p = LarkPlatform::Lark;
        assert_eq!(p.api_base(), LARK_BASE_URL);
        assert_eq!(p.ws_base(), LARK_WS_BASE_URL);
        assert_eq!(p.channel_name(), "lark");
        assert_eq!(p.proxy_service_key(), "channel.lark");
    }

    #[test]
    fn feishu_urls() {
        let p = LarkPlatform::Feishu;
        assert_eq!(p.api_base(), FEISHU_BASE_URL);
        assert_eq!(p.ws_base(), FEISHU_WS_BASE_URL);
        assert_eq!(p.channel_name(), "feishu");
        assert_eq!(p.proxy_service_key(), "channel.feishu");
    }

    #[test]
    fn api_base_differs_between_platforms() {
        assert_ne!(
            LarkPlatform::Lark.api_base(),
            LarkPlatform::Feishu.api_base()
        );
    }
}
