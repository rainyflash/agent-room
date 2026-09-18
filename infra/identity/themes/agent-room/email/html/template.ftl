<#-- 邮件客户端不读外部样式表：只用表格布局和行内样式。 -->
<#macro emailLayout>
<!DOCTYPE html>
<html lang="${locale.language}" dir="${(ltr)?then('ltr','rtl')}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light">
</head>
<body style="margin:0;padding:0;background:#FFF4DC;color:#2A2733;font-family:'Fredoka','PingFang SC','Microsoft YaHei','Helvetica Neue',Arial,sans-serif;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#FFF4DC;">
    <tr>
        <td align="center" style="padding:32px 16px;">
            <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:560px;">
                <tr>
                    <td align="center" style="padding:0 0 20px;font-size:21px;font-weight:700;letter-spacing:0.2px;">
                        <span style="display:inline-block;width:28px;height:28px;margin-right:8px;vertical-align:middle;border:2px solid #2A2733;border-radius:9px;background:#247A77;box-sizing:border-box;padding:6px;"><span style="display:block;width:12px;height:12px;border:2.5px solid #FFFDF7;border-radius:4px;"></span></span>
                        <span style="vertical-align:middle;">Agent Room</span>
                    </td>
                </tr>
                <tr>
                    <td style="padding:36px 36px 32px;background:#FFFDF7;border:2.5px solid #2A2733;border-radius:26px;box-shadow:0 5px 0 #2A2733;font-size:16px;line-height:1.65;">
                        <#nested>
                    </td>
                </tr>
                <tr>
                    <td align="center" style="padding:20px 12px 0;color:#5C5866;font-size:13px;line-height:1.6;">
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
    <h1 style="margin:0 0 16px;font-size:28px;line-height:1.25;font-weight:700;">${title}</h1>
    <p style="margin:0 0 12px;">${greeting}</p>
    <p style="margin:0 0 24px;">${body}</p>
    <p style="margin:0 0 24px;text-align:center;">
        <a href="${link}" style="display:inline-block;padding:14px 34px;border:2.5px solid #2A2733;border-radius:999px;background:#FFC53D;color:#2A2733;box-shadow:0 4px 0 #2A2733;font-size:17px;font-weight:700;text-decoration:none;">${button}</a>
    </p>
    <p style="margin:0 0 16px;color:#5C5866;font-size:14px;text-align:center;">${expiresIn}</p>
    <div style="margin:0 0 20px;padding:12px 14px;border:2px dashed #E8D9B8;border-radius:14px;background:#FFF6E3;font-size:13px;line-height:1.5;">
        <div style="margin:0 0 6px;color:#5C5866;">${msg("arEmailFallback")}</div>
        <div style="font-family:'SFMono-Regular',Consolas,monospace;font-size:12px;line-height:1.45;word-break:break-all;"><a href="${link}" style="color:#5C5866;">${link}</a></div>
    </div>
    <p style="margin:0;color:#5C5866;font-size:14px;">${ignore}</p>
</#macro>
