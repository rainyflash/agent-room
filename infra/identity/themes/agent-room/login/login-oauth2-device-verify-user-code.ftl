<#import "template.ftl" as layout>
<@layout.registrationLayout; section>
    <#if section = "header">
        ${msg("oauth2DeviceVerificationTitle")}
    <#elseif section = "form">
        <p class="ar-lead">${msg("arDeviceCodeLead")}</p>
        <form id="kc-user-verify-device-user-code-form" class="ar-form" action="${url.oauth2DeviceVerificationAction}" method="post">
            <div class="ar-field">
                <label for="device-user-code" class="ar-label">${msg("verifyOAuth2DeviceUserCode")}</label>
                <input id="device-user-code" name="device_user_code" class="ar-input ar-input--code" type="text"
                       autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="ABCD-EFGH" autofocus dir="ltr"/>
            </div>
            <button class="ar-button ar-button--primary ar-button--block" type="submit">
                ${msg("arNext")}
                <svg class="ar-button__arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M5 12h14M13 6l6 6-6 6"/></svg>
            </button>
        </form>
        <script>
            // Remember the typed code for this tab so the approval page can show it again.
            document.getElementById("kc-user-verify-device-user-code-form").addEventListener("submit", function () {
                try {
                    sessionStorage.setItem("agent-room-device-code", document.getElementById("device-user-code").value);
                } catch (ignored) {
                }
            });
        </script>
    </#if>
</@layout.registrationLayout>
