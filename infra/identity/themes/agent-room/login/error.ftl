<#import "template.ftl" as layout>
<@layout.registrationLayout displayMessage=false; section>
    <#if section = "header">
        ${kcSanitize(msg("errorTitle"))?no_esc}
    <#elseif section = "form">
        <div id="kc-error-message" class="ar-form">
            <p class="ar-alert ar-alert--error" role="alert">
                <span class="ar-alert__icon" aria-hidden="true"></span>
                <span class="ar-alert__text">${kcSanitize(message.summary)?no_esc}</span>
            </p>
            <#if traceId??>
                <p class="ar-hint" id="traceId">${msg("traceIdSupportMessage", traceId)}</p>
            </#if>
            <#if skipLink??>
            <#else>
                <#if client?? && client.baseUrl?has_content>
                    <a class="ar-button ar-button--secondary ar-button--block" id="backToApplication" href="${client.baseUrl}">${msg("backToApplication")}</a>
                </#if>
            </#if>
        </div>
    </#if>
</@layout.registrationLayout>
