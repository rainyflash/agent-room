<#import "template.ftl" as layout>
<@layout.registrationLayout displayInfo=true displayMessage=!messagesPerField.existsError('username'); section>
    <#if section = "header">
        ${msg("emailForgotTitle")}
    <#elseif section = "form">
        <p class="ar-lead"><#if realm.duplicateEmailsAllowed>${msg("emailInstructionUsername")}<#else>${msg("arResetPasswordLead")}</#if></p>
        <form id="kc-reset-password-form" class="ar-form" action="${url.loginAction}" method="post">
            <div class="ar-field">
                <label for="username" class="ar-label"><#if !realm.loginWithEmailAllowed>${msg("username")}<#elseif !realm.registrationEmailAsUsername>${msg("usernameOrEmail")}<#else>${msg("email")}</#if></label>
                <input id="username" class="ar-input" name="username" type="text" value="${(auth.attemptedUsername!'')}" autofocus
                       inputmode="email" autocapitalize="none" spellcheck="false" autocomplete="username" placeholder="you@example.com"
                       aria-invalid="<#if messagesPerField.existsError('username')>true</#if>" dir="ltr"/>
                <#if messagesPerField.existsError('username')>
                    <span id="input-error-username" class="ar-field-error" aria-live="polite">${kcSanitize(messagesPerField.get('username'))?no_esc}</span>
                </#if>
            </div>
            <button class="ar-button ar-button--primary ar-button--block" type="submit">${msg("arSendResetEmail")}</button>
        </form>
    <#elseif section = "info">
        <a class="ar-link" href="${url.loginUrl}">${msg("backToLogin")}</a>
    </#if>
</@layout.registrationLayout>
