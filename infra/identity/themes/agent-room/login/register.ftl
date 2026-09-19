<#import "template.ftl" as layout>
<#import "register-commons.ftl" as registerCommons>
<#-- 邮箱、昵称、密码。昵称存在 firstName 里，控制平面按 name 声明显示；姓氏在本 realm 的用户资料里只对管理员可见。 -->
<#assign emailValue = "">
<#assign nicknameValue = "">
<#assign collectsLocale = false>
<#list profile.attributes as attribute>
    <#if attribute.name == "email"><#assign emailValue = attribute.value!""></#if>
    <#if attribute.name == "firstName"><#assign nicknameValue = attribute.value!""></#if>
    <#if attribute.name == "locale"><#assign collectsLocale = true></#if>
</#list>
<@layout.registrationLayout displayMessage=messagesPerField.exists('global'); section>
    <#if section = "header">
        <#if messageHeader??>
            ${kcSanitize(msg("${messageHeader}"))?no_esc}
        <#else>
            ${msg("registerTitle")}
        </#if>
    <#elseif section = "form">
        <p class="ar-lead">${msg("arRegisterLead")}</p>
        <form id="kc-register-form" class="ar-form" action="${url.registrationAction}" method="post">
            <#if collectsLocale && realm.internationalizationEnabled && locale.currentLanguageTag?has_content>
                <input type="hidden" id="locale" name="locale" value="${locale.currentLanguageTag}"/>
            </#if>

            <div class="ar-field">
                <label for="email" class="ar-label">${msg("email")}</label>
                <input id="email" class="ar-input" name="email" type="email" value="${emailValue}" autofocus
                       autocomplete="email" inputmode="email" autocapitalize="none" spellcheck="false" placeholder="you@example.com"
                       aria-invalid="<#if messagesPerField.existsError('email')>true</#if>"
                       <#if messagesPerField.existsError('email')>aria-describedby="input-error-email"</#if> dir="ltr"/>
                <#if messagesPerField.existsError('email')>
                    <span id="input-error-email" class="ar-field-error" aria-live="polite">${kcSanitize(messagesPerField.get('email'))?no_esc}</span>
                </#if>
            </div>

            <div class="ar-field">
                <label for="firstName" class="ar-label">${msg("arNickname")}</label>
                <input id="firstName" class="ar-input" name="firstName" type="text" value="${nicknameValue}"
                       autocomplete="nickname" maxlength="60" placeholder="${msg("arNicknamePlaceholder")}"
                       aria-invalid="<#if messagesPerField.existsError('firstName')>true</#if>"
                       aria-describedby="<#if messagesPerField.existsError('firstName')>input-error-firstName <#else></#if>firstName-hint"/>
                <span id="firstName-hint" class="ar-hint">${msg("arNicknameHint")}</span>
                <#if messagesPerField.existsError('firstName')>
                    <span id="input-error-firstName" class="ar-field-error" aria-live="polite">${kcSanitize(messagesPerField.get('firstName'))?no_esc}</span>
                </#if>
            </div>

            <#if passwordRequired??>
                <div class="ar-field">
                    <label for="password" class="ar-label">${msg("password")}</label>
                    <div class="ar-password" dir="ltr">
                        <input id="password" class="ar-input" name="password" type="password" autocomplete="new-password"
                               aria-invalid="<#if messagesPerField.existsError('password','password-confirm')>true</#if>"
                               aria-describedby="<#if messagesPerField.existsError('password','password-confirm')>input-error-password </#if>password-hint"/>
                        <button class="ar-password-toggle" type="button" aria-label="${msg('showPassword')}"
                                aria-controls="password" data-password-toggle
                                data-icon-show="${properties.kcFormPasswordVisibilityIconShow!}" data-icon-hide="${properties.kcFormPasswordVisibilityIconHide!}"
                                data-label-show="${msg('showPassword')}" data-label-hide="${msg('hidePassword')}">
                            <i class="${properties.kcFormPasswordVisibilityIconShow!}" aria-hidden="true"></i>
                        </button>
                    </div>
                    <span id="password-hint" class="ar-hint">${msg("arPasswordHint")}</span>
                    <#if messagesPerField.existsError('password','password-confirm')>
                        <span id="input-error-password" class="ar-field-error" aria-live="polite">${kcSanitize(messagesPerField.getFirstError('password','password-confirm'))?no_esc}</span>
                    </#if>
                </div>
                <noscript>
                    <div class="ar-field">
                        <label for="password-confirm" class="ar-label">${msg("passwordConfirm")}</label>
                        <input id="password-confirm" class="ar-input" name="password-confirm" type="password" autocomplete="new-password"/>
                    </div>
                </noscript>
            </#if>

            <@registerCommons.termsAcceptance/>

            <button class="ar-button ar-button--primary ar-button--block" type="submit">
                ${msg("doRegister")}
                <svg class="ar-button__arrow" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M5 12h14M13 6l6 6-6 6"/></svg>
            </button>

            <div class="ar-signup">
                <span>${msg("arHaveAccount")}</span>
                <a class="ar-button ar-button--secondary ar-button--compact" href="${url.loginUrl}">${msg("doLogIn")}</a>
            </div>
        </form>
        <#if passwordRequired??>
            <script>
                // One visible password field: Keycloak still expects the confirmation, so it mirrors the field.
                (function () {
                    var form = document.getElementById("kc-register-form");
                    var password = document.getElementById("password");
                    var confirmation = document.createElement("input");
                    confirmation.type = "hidden";
                    confirmation.name = "password-confirm";
                    form.appendChild(confirmation);
                    var mirror = function () { confirmation.value = password.value; };
                    password.addEventListener("input", mirror);
                    form.addEventListener("submit", mirror);
                })();
            </script>
        </#if>
        <script type="module" src="${url.resourcesPath}/js/passwordVisibility.js"></script>
    </#if>
</@layout.registrationLayout>
