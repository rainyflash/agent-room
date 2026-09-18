<#import "template.ftl" as layout>
<#-- 设备登录一定经过这一页。本机连接（Bridge）用产品化文案，并请用户核对电脑上显示的码。 -->
<#assign isDevice = client.clientId == "agent-room-bridge">
<@layout.registrationLayout bodyClass="oauth"; section>
    <#if section = "header">
        <#if isDevice>
            ${msg("arDeviceGrantTitle")}
        <#elseif client.name?has_content>
            ${msg("oauthGrantTitle", advancedMsg(client.name))}
        <#else>
            ${msg("oauthGrantTitle", client.clientId)}
        </#if>
    <#elseif section = "form">
        <#if isDevice>
            <p class="ar-lead">${msg("arDeviceGrantLead")}</p>
            <div class="ar-code" id="ar-device-code" hidden>
                <span class="ar-code__label">${msg("arDeviceGrantCompare")}</span>
                <span class="ar-code__value" id="ar-device-code-value"></span>
            </div>
            <dl class="ar-facts">
                <div>
                    <dt>${msg("arDeviceGrantApp")}</dt>
                    <dd>${msg("arDeviceGrantAppName")}</dd>
                </div>
                <div>
                    <dt>${msg("arDeviceGrantAllows")}</dt>
                    <dd>${msg("arDeviceGrantAllowsValue")}</dd>
                </div>
            </dl>
            <p class="ar-callout">${msg("arDeviceGrantWarning")}</p>
        <#else>
            <p class="ar-lead">${msg("oauthGrantRequest")}</p>
            <#if oauth.clientScopesRequested??>
                <ul class="ar-scopes">
                    <#list oauth.clientScopesRequested as clientScope>
                        <li>
                            ${advancedMsg(clientScope.consentScreenText)}<#if clientScope.parameterizedScopeParameter??>: <b>${clientScope.parameterizedScopeParameter}</b></#if>
                        </li>
                    </#list>
                </ul>
            </#if>
        </#if>

        <form id="kc-oauth" class="ar-form" action="${url.oauthAction}" method="POST">
            <input type="hidden" name="code" value="${oauth.code}">
            <div class="ar-actions">
                <button class="ar-button ar-button--primary" name="accept" id="kc-login" type="submit">
                    <#if isDevice>${msg("arDeviceGrantApprove")}<#else>${msg("doYes")}</#if>
                </button>
                <button class="ar-button ar-button--secondary" name="cancel" id="kc-cancel" type="submit">
                    <#if isDevice>${msg("arDeviceGrantDeny")}<#else>${msg("doNo")}</#if>
                </button>
            </div>
        </form>
        <#if isDevice>
            <script>
                (function () {
                    var code = null;
                    try {
                        code = sessionStorage.getItem("agent-room-device-code");
                    } catch (ignored) {
                    }
                    var compact = (code || "").toUpperCase().replace(/[^A-Z0-9]/g, "");
                    if (compact.length >= 4 && compact.length <= 16) {
                        var half = Math.ceil(compact.length / 2);
                        document.getElementById("ar-device-code-value").textContent =
                            compact.length === 8 ? compact.slice(0, half) + "-" + compact.slice(half) : compact;
                        document.getElementById("ar-device-code").hidden = false;
                    }
                    document.getElementById("kc-oauth").addEventListener("submit", function () {
                        try {
                            sessionStorage.removeItem("agent-room-device-code");
                        } catch (ignored) {
                        }
                    });
                })();
            </script>
        </#if>
    </#if>
</@layout.registrationLayout>
