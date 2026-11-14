<#-- 邮件客户端不读外部样式表：只用表格布局和行内样式。 -->
<#macro emailLayout>
<!DOCTYPE html>
<html lang="${locale.language}" dir="${(ltr)?then('ltr','rtl')}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light">
</head>
<body style="margin:0;padding:0;background:#FFFFFF;color:#18181B;font-family:-apple-system,'Segoe UI','PingFang SC','Microsoft YaHei','Helvetica Neue',Arial,sans-serif;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#FFFFFF;">
    <tr>
        <td align="center" style="padding:32px 16px;">
            <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:560px;">
                <tr>
                    <td style="padding:0 0 20px;font-size:17px;font-weight:600;">
                        <span style="display:inline-block;width:28px;height:28px;margin-right:8px;vertical-align:middle;border-radius:8px;background:#247A77;box-sizing:border-box;padding:8px;"><span style="display:block;width:12px;height:12px;box-sizing:border-box;border:2.5px solid #FFFFFF;border-radius:4px;"></span></span>
                        <span style="vertical-align:middle;">Agent Room</span>
                    </td>
                </tr>
                <tr>
                    <td style="padding:32px;border:1px solid #DCDCE1;border-radius:16px;font-size:15px;line-height:1.65;">
                        <#nested>
                    </td>
                </tr>
                <tr>
                    <td style="padding:20px 4px 0;color:#686873;font-size:13px;line-height:1.6;">
                        ${msg("arEmailFooter")}
                    </td>
                </tr>
            </table>
        </td>
    </tr>
</table>
</body>
</html>
</#macro>

<#macro actionEmail title greeting body button link expiresIn ignore>
    <h1 style="margin:0 0 16px;font-size:24px;line-height:1.25;font-weight:600;">${title}</h1>
    <p style="margin:0 0 12px;">${greeting}</p>
    <p style="margin:0 0 24px;">${body}</p>
    <p style="margin:0 0 12px;">
        <a href="${link}" style="display:inline-block;padding:12px 24px;border-radius:8px;background:#18181B;color:#FFFFFF;font-size:15px;font-weight:600;text-decoration:none;">${button}</a>
    </p>
    <p style="margin:0 0 20px;color:#686873;font-size:13px;">${expiresIn}</p>
    <div style="margin:0 0 20px;padding:12px 14px;border-radius:8px;background:#F4F4F5;font-size:13px;line-height:1.5;">
        <div style="margin:0 0 6px;color:#686873;">${msg("arEmailFallback")}</div>
        <div style="font-family:'SFMono-Regular',Consolas,monospace;font-size:12px;line-height:1.45;word-break:break-all;"><a href="${link}" style="color:#686873;">${link}</a></div>
    </div>
    <p style="margin:0;color:#686873;font-size:13px;">${ignore}</p>
</#macro>
