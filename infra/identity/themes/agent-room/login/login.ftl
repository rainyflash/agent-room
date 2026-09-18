<#import "template.ftl" as layout>
<#import "passkeys.ftl" as passkeys>
<@layout.registrationLayout displayMessage=!messagesPerField.existsError('username','password') displayInfo=realm.password && realm.registrationAllowed && !registrationDisabled??; section>
    <#if section = "header">
        ${msg("loginAccountTitle")}
    <#elseif section = "form">
        <p class="ar-lead">${msg("arLoginLead")}</p>
        <#if realm.password>
            <form id="kc-form-login" class="ar-form" onsubmit="login.disabled = true; return true;" action="${url.loginAction}" method="post">
                <#if !usernameHidden??>
                    <div class="ar-field">
                        <label for="username" class="ar-label"><#if !realm.loginWithEmailAllowed>${msg("username")}<#elseif !realm.registrationEmailAsUsername>${msg("usernameOrEmail")}<#else>${msg("email")}</#if></label>
                        <input id="username" class="ar-input" name="username" value="${(login.username!'')}" type="text"
                               inputmode="email" autocapitalize="none" spellcheck="false" placeholder="you@example.com"
                               autofocus autocomplete="${(enableWebAuthnConditionalUI?has_content)?then('username webauthn', 'username')}"
                               aria-invalid="<#if messagesPerField.existsError('username','password')>true</#if>"
                               <#if messagesPerField.existsError('username','password')>aria-describedby="input-error"</#if>
                               dir="ltr"/>
                        <#if messagesPerField.existsError('username','password')>
                            <span id="input-error" class="ar-field-error" aria-live="polite">
                                ${kcSanitize(messagesPerField.getFirstError('username','password'))?no_esc}
                            </span>
                        </#if>
                    </div>
                </#if>

                <div class="ar-field">
                    <label for="password" class="ar-label">${msg("password")}</label>
                    <div class="ar-password" dir="ltr">
                        <input id="password" class="ar-input" name="password" type="password" autocomplete="current-password"
                               aria-invalid="<#if messagesPerField.existsError('username','password')>true</#if>"/>
                        <button class="ar-password-toggle" type="button" aria-label="${msg("showPassword")}"
                                aria-controls="password" data-password-toggle
                                data-icon-show="${properties.kcFormPasswordVisibilityIconShow!}" data-icon-hide="${properties.kcFormPasswordVisibilityIconHide!}"
                                data-label-show="${msg('showPassword')}" data-label-hide="${msg('hidePassword')}">
                            <i class="${properties.kcFormPasswordVisibilityIconShow!}" aria-hidden="true"></i>
                        </button>
                    </div>
                    <#if usernameHidden?? && messagesPerField.existsError('username','password')>
                        <span id="input-error" class="ar-field-error" aria-live="polite">
                            ${kcSanitize(messagesPerField.getFirstError('username','password'))?no_esc}
                        </span>
                    </#if>
                </div>

                <div class="ar-form-row">
                    <#if realm.rememberMe && !usernameHidden??>
                        <label class="ar-check" for="rememberMe">
                            <input id="rememberMe" name="rememberMe" type="checkbox"<#if login.rememberMe??> checked</#if>>
                            ${msg("rememberMe")}
                        </label>
                    <#else>
                        <span></span>
                    </#if>
                    <#if realm.resetPasswordAllowed>
                        <a class="ar-link" href="${url.loginResetCredentialsUrl}">${msg("doForgotPassword")}</a>
                    </#if>
                </div>

                <input type="hidden" id="id-hidden-input" name="credentialId" <#if auth.selectedCredential?has_content>value="${auth.selectedCredential}"</#if>/>
                <button class="ar-button ar-button--primary ar-button--block" name="login" id="kc-login" type="submit">
                    ${msg("doLogIn")}
                    <svg class="ar-button__arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M5 12h14M13 6l6 6-6 6"/></svg>
                </button>
            </form>
        </#if>
        <@passkeys.conditionalUIData />
        <script type="module" src="${url.resourcesPath}/js/passwordVisibility.js"></script>
    <#elseif section = "info">
        <#if realm.password && realm.registrationAllowed && !registrationDisabled??>
            <span id="kc-registration">${msg("noAccount")}</span>
            <a class="ar-button ar-button--secondary ar-button--compact" href="${url.registrationUrl}">${msg("doRegister")}</a>
        </#if>
    <#elseif section = "socialProviders">
        <#if realm.password && social?? && social.providers?has_content>
            <div id="kc-social-providers" class="ar-social">
                <p class="ar-hint">${msg("identity-provider-login-label")}</p>
                <ul class="ar-social__list">
                    <#list social.providers as p>
                        <li>
                            <a data-once-link id="social-${p.alias}" class="ar-button ar-button--secondary ar-button--block" href="${p.loginUrl}">${p.displayName!}</a>
                        </li>
                    </#list>
                </ul>
            </div>
        </#if>
    </#if>
</@layout.registrationLayout>
