<#import "template.ftl" as layout>
<#import "password-commons.ftl" as passwordCommons>
<#-- Keycloak's "update your password" warning repeats this page's own lead. -->
<@layout.registrationLayout displayMessage=!messagesPerField.existsError('password','password-confirm') && !(message?? && message.type == 'warning'); section>
    <#if section = "header">
        <#if isAppInitiatedAction??>${msg("arChangePasswordTitle")}<#else>${msg("updatePasswordTitle")}</#if>
    <#elseif section = "form">
        <#-- 从应用的“修改密码”来的（specs/change-password/design.md）：改好以后别处的登录一律退出，
             不给勾选框。Keycloak 默认不勾，不勾的话别的浏览器在这里的登录还在，悄悄就能再登录回来。 -->
        <p class="ar-lead"><#if isAppInitiatedAction??>${msg("arChangePasswordLead")}<#else>${msg("arUpdatePasswordLead")}</#if></p>
        <form id="kc-passwd-update-form" class="ar-form" onsubmit="login.disabled = true; return true;" action="${url.loginAction}" method="post">
            <#list ["password-new", "password-confirm"] as field>
                <div class="ar-field">
                    <label for="${field}" class="ar-label"><#if field == "password-new">${msg("passwordNew")}<#else>${msg("passwordConfirm")}</#if></label>
                    <div class="ar-password" dir="ltr">
                        <input id="${field}" class="ar-input" name="${field}" type="password" autocomplete="new-password"<#if field == "password-new"> autofocus aria-describedby="password-hint"</#if>
                               aria-invalid="<#if messagesPerField.existsError('password','password-confirm')>true</#if>"/>
                        <button class="ar-password-toggle" type="button" aria-label="${msg('showPassword')}"
                                aria-controls="${field}" data-password-toggle
                                data-icon-show="${properties.kcFormPasswordVisibilityIconShow!}" data-icon-hide="${properties.kcFormPasswordVisibilityIconHide!}"
                                data-label-show="${msg('showPassword')}" data-label-hide="${msg('hidePassword')}">
                            <i class="${properties.kcFormPasswordVisibilityIconShow!}" aria-hidden="true"></i>
                        </button>
                    </div>
                    <#if field == "password-new">
                        <span id="password-hint" class="ar-hint">${msg("arPasswordHint")}</span>
                    </#if>
                </div>
            </#list>
            <#if messagesPerField.existsError('password','password-confirm')>
                <span id="input-error-password" class="ar-field-error" aria-live="polite">${kcSanitize(messagesPerField.getFirstError('password','password-confirm'))?no_esc}</span>
            </#if>

            <#if isAppInitiatedAction??>
                <input type="hidden" name="logout-sessions" value="on"/>
            <#else>
                <@passwordCommons.logoutOtherSessions/>
            </#if>

            <#if isAppInitiatedAction??>
                <div class="ar-actions">
                    <button class="ar-button ar-button--primary" name="login" type="submit">${msg("arSavePassword")}</button>
                    <button class="ar-button ar-button--secondary" type="submit" name="cancel-aia" value="true">${msg("doCancel")}</button>
                </div>
            <#else>
                <button class="ar-button ar-button--primary ar-button--block" name="login" type="submit">${msg("arSavePassword")}</button>
            </#if>
        </form>
        <script type="module" src="${url.resourcesPath}/js/passwordVisibility.js"></script>
    </#if>
</@layout.registrationLayout>
