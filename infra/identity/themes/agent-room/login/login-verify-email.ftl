<#import "template.ftl" as layout>
<#-- Keycloak's "verify your email" warning repeats this page's own text; other messages (e.g. resent) still show. -->
<@layout.registrationLayout displayInfo=!isAppInitiatedAction?? displayMessage=!(message?? && message.type == 'warning'); section>
    <#if section = "header">
        ${msg("emailVerifyTitle")}
    <#elseif section = "form">
        <#assign address = verifyEmail!(user.email!"")>
        <#if !isAppInitiatedAction??>
            <span class="ar-chip">${msg("arAccountCreated")}</span>
        </#if>
        <p class="ar-lead">${kcSanitize(msg("arVerifyEmailLead", address))?no_esc}</p>
        <ol class="ar-steps">
            <li>${msg("arVerifyEmailStep1")}</li>
            <li>${msg("arVerifyEmailStep2")}</li>
            <li>${msg("arVerifyEmailStep3")}</li>
        </ol>
        <p class="ar-callout">${msg("arVerifyEmailSpam")}</p>
        <#if isAppInitiatedAction??>
            <form id="kc-verify-email-form" class="ar-form" action="${url.loginAction}" method="post">
                <div class="ar-form-buttons">
                    <#if verifyEmail??>
                        <button class="ar-button ar-button--secondary ar-button--block" type="submit">${msg("emailVerifyResend")}</button>
                    <#else>
                        <button class="ar-button ar-button--primary ar-button--block" type="submit">${msg("emailVerifySend")}</button>
                    </#if>
                    <button class="ar-button ar-button--secondary ar-button--block" type="submit" name="cancel-aia" value="true" formnovalidate>${msg("doCancel")}</button>
                </div>
            </form>
        </#if>
    <#elseif section = "info">
        <a class="ar-button ar-button--secondary ar-button--compact" href="${url.loginAction}">${msg("arResendEmail")}</a>
        <#if realm.registrationAllowed>
            <a class="ar-link" href="${url.registrationUrl}">${msg("arUseAnotherEmail")}</a>
        </#if>
    </#if>
</@layout.registrationLayout>
